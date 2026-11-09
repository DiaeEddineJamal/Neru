//! What this computer can run locally: memory, processor, graphics cards and free disk space, matched
//! against the local-model catalog in the model registry. On most PCs local models are helpers next to
//! free cloud models (titles, summaries, commit messages, repairs, completion); the agent itself runs
//! locally only where there is the memory for a 20–30B-class model. See LOCAL_MODELS_STUDY.md.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    /// Memory of its own; integrated graphics borrow system memory and count as none.
    pub vram_gb: f32,
    pub dedicated: bool,
    /// nvidia, amd, intel, apple or other: picks the built-in engine's backend (CUDA, Vulkan, Metal).
    pub vendor: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    pub ram_gb: f32,
    pub cpu: String,
    pub threads: usize,
    pub avx2: bool,
    pub avx512: bool,
    pub gpus: Vec<Gpu>,
    /// Apple silicon: the graphics share all of memory.
    pub unified: bool,
    /// Where Ollama keeps models, and the free space there.
    pub models_dir: String,
    pub disk_free_gb: Option<f32>,
}

/// A model in the registry's local catalog (`localModels`).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    /// The Ollama tag, such as `qwen3-coder:30b`.
    pub tag: String,
    /// Download size, which is about what it takes in memory before the context.
    pub size_gb: f32,
    /// Weights read for each generated token: the whole model when dense, the active experts when a
    /// mixture of experts. Memory speed divided by this is about how fast it writes.
    pub active_gb: f32,
    /// helper (chores), complete (tab completion), agent (can drive Code), embed (semantic search).
    pub roles: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub role: String,
    pub tag: String,
    pub size_gb: f32,
    /// "gpu" or "cpu".
    pub runs_on: String,
    /// A rough estimate from memory speed; the speed check after a download measures it.
    pub tokens_per_second: u32,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalAdvice {
    pub hardware: Hardware,
    pub picks: Vec<Pick>,
    /// Whether a local model can drive the agent here.
    pub agent: bool,
    /// One or two plain sentences.
    pub verdict: String,
}

// ---------- detection ----------

fn gb(bytes: f64) -> f32 {
    (bytes / 1_073_741_824.0) as f32
}

/// Integrated graphics report a small carve-out of system memory, which does not hold a model.
fn integrated(name: &str) -> bool {
    let lower = name.to_lowercase();
    ["iris", "uhd graphics", "hd graphics", "radeon(tm) graphics", "radeon graphics", "basic display", "basic render", "virtual", "parsec", "remote"].iter().any(|part| lower.contains(part))
}

fn ollama_models_dir() -> std::path::PathBuf {
    std::env::var_os("OLLAMA_MODELS").map(std::path::PathBuf::from).or_else(|| dirs::home_dir().map(|home| home.join(".ollama").join("models"))).unwrap_or_default()
}

#[cfg(windows)]
fn detect_platform(hardware: &mut Hardware, models_dir: &std::path::Path) {
    use crate::Hidden;
    // One PowerShell run: memory, processor name, graphics cards (the registry's 64-bit memory size,
    // since WMI's AdapterRAM stops at 4 GB, with AdapterRAM as the fallback) and free space on the
    // drive that holds Ollama's models.
    let drive = models_dir.to_string_lossy().chars().next().filter(|c| c.is_ascii_alphabetic()).unwrap_or('C');
    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue'
$cs = Get-CimInstance Win32_ComputerSystem
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$wmi = @{{}}; Get-CimInstance Win32_VideoController | ForEach-Object {{ $wmi[$_.Name] = [double]$_.AdapterRAM }}
$gpus = @(Get-ItemProperty 'HKLM:\SYSTEM\ControlSet001\Control\Class\{{4d36e968-e325-11ce-bfc1-08002be10318}}\0*' | Where-Object {{ $_.DriverDesc }} | ForEach-Object {{
  $q = $_.'HardwareInformation.qwMemorySize'; $v = if ($q) {{ [double]$q }} else {{ $wmi[$_.DriverDesc] }}
  @{{ name = $_.DriverDesc; vram = $v }} }})
if ($gpus.Count -eq 0) {{ $gpus = @($wmi.Keys | ForEach-Object {{ @{{ name = $_; vram = $wmi[$_] }} }}) }}
$free = (Get-PSDrive -Name '{drive}').Free
@{{ ram = [double]$cs.TotalPhysicalMemory; cpu = $cpu.Name; gpus = $gpus; free = $free }} | ConvertTo-Json -Compress -Depth 4"#
    );
    let mut command = std::process::Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    let Ok(output) = command.hidden().output() else { return };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else { return };
    hardware.ram_gb = gb(value["ram"].as_f64().unwrap_or(0.0));
    hardware.cpu = value["cpu"].as_str().unwrap_or_default().trim().to_string();
    hardware.disk_free_gb = value["free"].as_f64().map(gb);
    let list = match &value["gpus"] {
        serde_json::Value::Array(items) => items.clone(),
        single @ serde_json::Value::Object(_) => vec![single.clone()],
        _ => Vec::new(),
    };
    hardware.gpus = list.iter().map(|gpu| gpu_from(gpu["name"].as_str().unwrap_or_default(), gb(gpu["vram"].as_f64().unwrap_or(0.0)))).collect();
}

#[cfg(not(windows))]
fn detect_platform(hardware: &mut Hardware, models_dir: &std::path::Path) {
    let run = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).output().ok().filter(|out| out.status.success()).map(|out| String::from_utf8_lossy(&out.stdout).to_string());
    if cfg!(target_os = "macos") {
        hardware.ram_gb = run("sysctl", &["-n", "hw.memsize"]).and_then(|text| text.trim().parse::<f64>().ok()).map(gb).unwrap_or(0.0);
        hardware.cpu = run("sysctl", &["-n", "machdep.cpu.brand_string"]).unwrap_or_default().trim().to_string();
        hardware.unified = cfg!(target_arch = "aarch64");
    } else {
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        hardware.ram_gb = meminfo.lines().find(|line| line.starts_with("MemTotal:")).and_then(|line| line.split_whitespace().nth(1)?.parse::<f64>().ok()).map(|kb| gb(kb * 1024.0)).unwrap_or(0.0);
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        hardware.cpu = cpuinfo.lines().find(|line| line.starts_with("model name")).and_then(|line| line.split_once(':')).map(|(_, name)| name.trim().to_string()).unwrap_or_default();
    }
    // NVIDIA cards on Linux (and the rare Intel Mac with one) report through nvidia-smi.
    if let Some(text) = run("nvidia-smi", &["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]) {
        hardware.gpus = text.lines().filter_map(|line| line.rsplit_once(',')).map(|(name, mib)| gpu_from(name.trim(), mib.trim().parse::<f32>().unwrap_or(0.0) / 1024.0)).collect();
    }
    let existing = std::iter::successors(Some(models_dir), |path| path.parent()).find(|path| path.exists()).map(|path| path.to_string_lossy().to_string()).unwrap_or_else(|| "/".into());
    hardware.disk_free_gb = run("df", &["-Pk", &existing]).and_then(|text| text.lines().nth(1)?.split_whitespace().nth(3)?.parse::<f64>().ok()).map(|kb| gb(kb * 1024.0));
}

pub fn vendor_of(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if ["nvidia", "geforce", "rtx", "quadro", "tesla"].iter().any(|part| lower.contains(part)) {
        "nvidia"
    } else if ["amd", "radeon"].iter().any(|part| lower.contains(part)) {
        "amd"
    } else if ["intel", "arc"].iter().any(|part| lower.contains(part)) {
        "intel"
    } else if lower.contains("apple") {
        "apple"
    } else {
        "other"
    }
}

pub fn gpu_from(name: &str, vram_gb: f32) -> Gpu {
    let dedicated = !integrated(name) && vram_gb >= 2.0;
    Gpu { name: name.to_string(), vram_gb: if dedicated { vram_gb } else { 0.0 }, dedicated, vendor: vendor_of(name).into() }
}

/// Free space in GB on the drive that holds `dir` (or its nearest existing parent).
pub fn free_space(dir: &std::path::Path) -> Option<f32> {
    let existing = std::iter::successors(Some(dir), |path| path.parent()).find(|path| path.exists())?;
    #[cfg(windows)]
    {
        use crate::Hidden;
        let drive = existing.to_string_lossy().chars().next().filter(|c| c.is_ascii_alphabetic())?;
        let mut command = std::process::Command::new("powershell");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &format!("(Get-PSDrive -Name '{drive}').Free")]);
        let output = command.hidden().output().ok()?;
        String::from_utf8_lossy(&output.stdout).trim().parse::<f64>().ok().map(gb)
    }
    #[cfg(not(windows))]
    {
        let output = std::process::Command::new("df").args(["-Pk", &existing.to_string_lossy()]).output().ok()?;
        String::from_utf8_lossy(&output.stdout).lines().nth(1)?.split_whitespace().nth(3)?.parse::<f64>().ok().map(|kb| gb(kb * 1024.0))
    }
}

/// This computer, measured once per run (a PowerShell or sysctl call takes a moment).
pub fn detect() -> Hardware {
    static HARDWARE: OnceLock<Hardware> = OnceLock::new();
    HARDWARE
        .get_or_init(|| {
            let models_dir = ollama_models_dir();
            let mut hardware = Hardware { threads: std::thread::available_parallelism().map_or(1, |n| n.get()), models_dir: models_dir.to_string_lossy().to_string(), ..Hardware::default() };
            #[cfg(target_arch = "x86_64")]
            {
                hardware.avx2 = std::arch::is_x86_feature_detected!("avx2");
                hardware.avx512 = std::arch::is_x86_feature_detected!("avx512f");
            }
            detect_platform(&mut hardware, &models_dir);
            hardware
        })
        .clone()
}

// ---------- recommendations ----------

/// Memory a model may take on the processor: what is left after the system and the apps beside it.
fn ram_budget(hardware: &Hardware) -> f32 {
    if hardware.unified {
        // macOS lets the graphics use about two thirds of unified memory by default.
        return hardware.ram_gb * 0.65;
    }
    (hardware.ram_gb - 8.0).max(hardware.ram_gb * 0.5)
}

fn gpu_budget(hardware: &Hardware) -> f32 {
    if hardware.unified {
        return hardware.ram_gb * 0.65;
    }
    hardware.gpus.iter().filter(|gpu| gpu.dedicated).map(|gpu| gpu.vram_gb * 0.9 - 0.5).fold(0.0, f32::max)
}

/// Memory speed in GB/s a token's weights are read at: a typical card, Apple silicon, or dual-channel
/// laptop and desktop memory (less without AVX2).
fn bandwidth(on_gpu: bool, hardware: &Hardware) -> f32 {
    match (on_gpu, hardware.unified) {
        (true, true) => 100.0,
        (true, false) => 250.0,
        (false, _) if hardware.avx2 => 28.0,
        (false, _) => 12.0,
    }
}

/// The weights plus room for the context and the runtime.
fn footprint(model: &LocalModel) -> f32 {
    // About 1 GB for a 16K context with a q8 cache, and half a gigabyte for the runtime.
    model.size_gb + 1.5
}

/// Fewest tokens a second each role is worth running at: chores must be quick, the agent can be slower.
fn min_speed(role: &str) -> u32 {
    match role {
        "complete" => 20,
        // Chores write a line or two, so a modest speed is fine.
        "helper" => 8,
        "agent" => 8,
        _ => 0,
    }
}

pub fn recommend(hardware: &Hardware, catalog: &[LocalModel]) -> LocalAdvice {
    let (gpu, ram) = (gpu_budget(hardware), ram_budget(hardware));
    let mut picks = Vec::new();
    for role in ["agent", "helper", "complete", "embed"] {
        // The catalog lists each role's models best first.
        let pick = catalog.iter().filter(|model| model.roles.iter().any(|r| r == role)).find_map(|model| {
            let on_gpu = footprint(model) <= gpu;
            if !on_gpu && footprint(model) > ram {
                return None;
            }
            let speed = (bandwidth(on_gpu, hardware) / model.active_gb.max(0.1)) as u32;
            (speed >= min_speed(role)).then(|| Pick { role: role.into(), tag: model.tag.clone(), size_gb: model.size_gb, runs_on: if on_gpu { "gpu" } else { "cpu" }.into(), tokens_per_second: speed })
        });
        picks.extend(pick);
    }
    let agent = picks.iter().find(|pick| pick.role == "agent");
    let mut verdict = match agent {
        Some(pick) if pick.runs_on == "gpu" => format!("This computer can run {} for the agent itself on its graphics card, with small helpers beside it.", pick.tag),
        Some(pick) => format!(
            "This computer can run {} for the agent itself, slowly: about {} tokens a second, and the first reply of a session can take a few minutes while it reads the prompt. Small helpers run well here.",
            pick.tag, pick.tokens_per_second
        ),
        None if picks.iter().any(|pick| pick.role == "helper") => "Local models fit here as helpers: titles, summaries, commit messages and repairs run on this computer and save your free requests. The agent itself is better on a free cloud model.".into(),
        None => "This computer is too small for local models to help much; free cloud models are the better choice.".into(),
    };
    if let Some(free) = hardware.disk_free_gb {
        let needed: f32 = picks.iter().map(|pick| pick.size_gb).sum();
        if needed + 2.0 > free {
            verdict.push_str(&format!(" The drive that holds Ollama's models has {free:.0} GB free; these need about {needed:.0} GB."));
        }
    }
    LocalAdvice { hardware: hardware.clone(), agent: agent.is_some(), picks, verdict }
}

// ---------- the built-in engine's recommendations ----------

/// One catalog model with the quantization chosen for this computer.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinPick {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub roles: Vec<String>,
    pub note: String,
    pub quant: String,
    pub file: String,
    pub bytes: u64,
    /// "gpu" (all layers on the card), "partial" (some on the card, the rest in memory), "cpu".
    pub runs_on: String,
    /// Share of the layers on the graphics card, 0 to 100.
    pub gpu_percent: u32,
    /// An estimate from memory speed until the speed check measures it.
    pub tokens_per_second: u32,
    /// "great", "good", "tight", "slow", "disk" (fits, but not on the drive) or "no".
    pub fit: String,
    pub fit_label: String,
    pub quality: String,
    /// Context the engine would start this model with here.
    pub context: u32,
    /// The best pick for one of its roles on this computer.
    pub recommended: bool,
    /// The other quantizations that fit, best first, for a manual choice.
    pub alternatives: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinAdvice {
    pub hardware: Hardware,
    /// cpu, vulkan, cuda or metal: what the engine downloads for this computer.
    pub backend: String,
    pub models: Vec<BuiltinPick>,
    pub verdict: String,
    pub disk_free_gb: Option<f32>,
}

/// How a quantization compares with the original weights, in plain words.
pub fn quality_label(quant: &str) -> &'static str {
    match quant {
        "Q8_0" | "F16" | "BF16" => "Near the original",
        "Q6_K" => "Excellent",
        "Q5_K_M" | "Q5_K_S" => "Very good",
        "Q4_K_M" | "Q4_K_S" | "Q4_0" | "MXFP4" => "Good",
        "IQ4_XS" | "IQ4_NL" => "Good, smaller",
        "Q3_K_M" | "Q3_K_S" => "Noticeably weaker",
        _ => "Weaker",
    }
}

/// The backend the built-in engine uses here: CUDA for an NVIDIA card, Vulkan for another card with its
/// own memory (2 GB or more), Metal on a Mac, and the processor otherwise. Integrated graphics stay on
/// the processor: Vulkan on them is rarely faster and sometimes unstable.
pub fn engine_backend(hardware: &Hardware) -> &'static str {
    if cfg!(target_os = "macos") {
        return if hardware.unified || cfg!(target_arch = "aarch64") { "metal" } else { "cpu" };
    }
    let best = hardware.gpus.iter().filter(|gpu| gpu.dedicated && gpu.vram_gb >= 2.0).max_by(|a, b| a.vram_gb.total_cmp(&b.vram_gb));
    match best {
        Some(gpu) if gpu.vendor == "nvidia" && cfg!(windows) => "cuda",
        Some(_) => "vulkan",
        None => "cpu",
    }
}

/// Bytes a value of the key/value cache takes: Neru runs it at q8_0 (8.5 bits).
const KV_BYTES: f32 = 8.5 / 8.0;

/// The context the engine starts a model with: as much as fits beside the weights, at most what the
/// model was trained for, 64K on a roomy machine and 32K on one with 16 GB or less. At least 4K.
pub fn plan_context(hardware: &Hardware, weights_gb: f32, kv_bytes_per_token: f32, trained: u32) -> u32 {
    let pool = gpu_budget(hardware).max(ram_budget(hardware));
    let cap = if hardware.ram_gb <= 16.5 { 32_768 } else { 65_536 };
    let room = (pool - weights_gb - 1.0).max(0.0) * 1_073_741_824.0;
    let fits = (room / kv_bytes_per_token.max(1.0)) as u32;
    let trained = if trained == 0 { 32_768 } else { trained };
    (fits.min(cap).min(trained) / 1024 * 1024).max(4096.min(trained))
}

/// Where a model of `size_gb` (weights plus cache) runs, the share of layers on the card and the
/// tokens a second to expect.
fn placement(hardware: &Hardware, size_gb: f32, active_gb: f32) -> Option<(&'static str, u32, u32)> {
    let (gpu, ram) = (gpu_budget(hardware), ram_budget(hardware));
    let need = size_gb + 0.5;
    let cpu_speed = bandwidth(false, hardware) / active_gb.max(0.05);
    if gpu > 0.0 && need <= gpu {
        return Some(("gpu", 100, (bandwidth(true, hardware) / active_gb.max(0.05)) as u32));
    }
    if gpu > 1.0 && !hardware.unified && need <= gpu + ram {
        // Time a token takes is the card's share at card speed plus the rest at memory speed.
        let share = (gpu / need).clamp(0.0, 1.0);
        let seconds = active_gb * share / bandwidth(true, hardware) + active_gb * (1.0 - share) / bandwidth(false, hardware);
        return Some(("partial", (share * 100.0) as u32, (1.0 / seconds.max(1e-4)) as u32));
    }
    (need <= ram).then_some(("cpu", 0, cpu_speed as u32))
}

pub fn builtin_recommend(hardware: &Hardware, catalog: &[crate::llama_models::GgufModel], disk_free_gb: Option<f32>) -> BuiltinAdvice {
    let backend = engine_backend(hardware);
    let mut models = Vec::new();
    for model in catalog {
        let role = if model.roles.iter().any(|r| r == "agent") { "agent" } else if model.roles.iter().any(|r| r == "helper") { "helper" } else { "chat" };
        // Chat is read as it streams, so it wants a little more than chores.
        let floor = if role == "chat" { 10 } else { min_speed(role).max(6) };
        // Each quantization with where it runs; the files are listed best first.
        let options: Vec<_> = model
            .files
            .iter()
            .filter_map(|file| {
                let weights = file.bytes as f32 / 1_073_741_824.0;
                let context = plan_context(hardware, weights, model.kv_kb_per_token * 1024.0 * KV_BYTES / 2.0, model.context_max);
                // A 16K working context for the fit; the engine may start with more.
                let kv = model.kv_kb_per_token * 1024.0 * KV_BYTES / 2.0 * 16_384.0 / 1_073_741_824.0;
                let bits = file.bytes as f32 * 8.0 / (model.params_b * 1e9);
                let active = model.active_b * bits / 8.0;
                placement(hardware, weights + kv, active).map(|(runs_on, percent, speed)| (file, runs_on, percent, speed, context, weights + kv))
            })
            .collect();
        let rank = |runs_on: &str| match runs_on {
            "gpu" => 0,
            "partial" => 1,
            _ => 2,
        };
        // The best place it can run, then the best quality there that is fast enough; Q3 only when
        // nothing better fits.
        let usable: Vec<_> = options.iter().filter(|option| !option.0.quant.starts_with("Q3")).collect();
        let usable = if usable.is_empty() { options.iter().collect() } else { usable };
        let tier = usable.iter().map(|option| (if option.3 >= floor { 0 } else { 1 }, rank(option.1))).min();
        let chosen = tier.and_then(|(slow, place)| {
            let in_tier: Vec<_> = usable.iter().filter(|option| rank(option.1) == place).collect();
            // Fast enough: the best quality that is. Too slow everywhere: the smallest, fastest file.
            // Comfortably fast first (half again the floor), so a few points of quality do not cost a
            // third of the speed; then just fast enough; then the smallest file.
            let comfortable = floor + floor / 2;
            if slow == 0 { in_tier.iter().find(|option| option.3 >= comfortable).or_else(|| in_tier.iter().find(|option| option.3 >= floor)).map(|option| ***option) } else { in_tier.last().map(|option| ***option) }
        });
        let Some((file, runs_on, percent, speed, context, need)) = chosen else {
            let smallest = model.files.last();
            models.push(BuiltinPick {
                id: model.id.clone(),
                name: model.name.clone(),
                repo: model.repo.clone(),
                roles: model.roles.clone(),
                note: model.note.clone(),
                quant: smallest.map(|f| f.quant.clone()).unwrap_or_default(),
                file: smallest.map(|f| f.file.clone()).unwrap_or_default(),
                bytes: smallest.map_or(0, |f| f.bytes),
                runs_on: "none".into(),
                gpu_percent: 0,
                tokens_per_second: 0,
                fit: "no".into(),
                fit_label: "Too large for this computer".into(),
                quality: String::new(),
                context: 0,
                recommended: false,
                alternatives: Vec::new(),
            });
            continue;
        };
        let pool = if runs_on == "gpu" { gpu_budget(hardware) } else { ram_budget(hardware) + if runs_on == "partial" { gpu_budget(hardware) } else { 0.0 } };
        let (fit, fit_label) = if speed < floor {
            ("slow", format!("Fits, but slow here (about {speed} tokens a second)"))
        } else if need > pool * 0.85 || runs_on == "partial" {
            ("tight", match runs_on {
                "partial" => format!("Runs, partly on the graphics card ({percent}% of layers), about {speed} tokens a second"),
                _ => format!("Runs, but tight on memory, about {speed} tokens a second"),
            })
        } else if speed >= floor * 3 {
            ("great", format!("Runs well {}, about {speed} tokens a second", if runs_on == "gpu" { "on the graphics card" } else { "on the processor" }))
        } else {
            ("good", format!("Runs {}, about {speed} tokens a second", if runs_on == "gpu" { "on the graphics card" } else { "on the processor" }))
        };
        let disk_short = disk_free_gb.is_some_and(|free| (file.bytes as f32 / 1_073_741_824.0) + 1.0 > free);
        models.push(BuiltinPick {
            id: model.id.clone(),
            name: model.name.clone(),
            repo: model.repo.clone(),
            roles: model.roles.clone(),
            note: model.note.clone(),
            quant: file.quant.clone(),
            file: file.file.clone(),
            bytes: file.bytes,
            runs_on: runs_on.into(),
            gpu_percent: percent,
            tokens_per_second: speed,
            fit: if disk_short { "disk".into() } else { fit.into() },
            fit_label: if disk_short { format!("{fit_label}, but the drive needs {:.0} GB free", file.bytes as f32 / 1_073_741_824.0 + 1.0) } else { fit_label },
            quality: quality_label(&file.quant).into(),
            context,
            recommended: false,
            alternatives: options.iter().filter(|option| option.0.quant != file.quant).map(|option| option.0.quant.clone()).collect(),
        });
    }
    // The first good pick per role in catalog order (best first) is the recommendation.
    for role in ["agent", "chat", "helper"] {
        let best = models.iter().position(|pick| pick.roles.iter().any(|r| r == role) && matches!(pick.fit.as_str(), "great" | "good" | "tight") && (role != "agent" || pick.tokens_per_second >= 8));
        if let Some(index) = best {
            models[index].recommended = true;
        }
    }
    let order = |pick: &BuiltinPick| (!pick.recommended, match pick.fit.as_str() { "great" => 0, "good" => 1, "tight" => 2, "slow" => 3, "disk" => 4, _ => 5 });
    models.sort_by_key(order);
    let agent = models.iter().find(|pick| pick.recommended && pick.roles.iter().any(|r| r == "agent"));
    let verdict = match agent {
        Some(pick) => format!("{} can drive Code on this computer ({}). Smaller models below handle chat and chores fast.", pick.name, pick.fit_label.to_lowercase()),
        None if models.iter().any(|pick| pick.recommended) => "Models up to a few billion parameters run well here, for chat and chores such as titles, summaries and commit messages. For Code itself, a free cloud model is faster.".into(),
        None => "This computer is small for local models; free cloud models are the better choice.".into(),
    };
    BuiltinAdvice { hardware: hardware.clone(), backend: backend.into(), models, verdict, disk_free_gb }
}

/// Catalog models with the quantization that fits this computer, for Settings.
#[tauri::command]
pub async fn builtin_advice() -> Result<BuiltinAdvice, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let hardware = detect();
        let free = crate::llama_models::models_dir().ok().and_then(|dir| free_space(&dir));
        builtin_recommend(&hardware, &crate::llama_models::catalog(), free)
    })
    .await
    .map_err(|e| e.to_string())
}

/// The local models that fit this computer, for Settings and `neru doctor`.
#[tauri::command]
pub async fn local_advice() -> Result<LocalAdvice, String> {
    let hardware = tauri::async_runtime::spawn_blocking(detect).await.map_err(|e| e.to_string())?;
    Ok(recommend(&hardware, &crate::registry::get().local_models))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<LocalModel> {
        crate::registry::get().local_models
    }

    fn machine(ram_gb: f32, gpus: Vec<Gpu>) -> Hardware {
        Hardware { ram_gb, avx2: true, threads: 8, gpus, disk_free_gb: Some(200.0), ..Hardware::default() }
    }

    fn tag<'a>(advice: &'a LocalAdvice, role: &str) -> Option<&'a str> {
        advice.picks.iter().find(|pick| pick.role == role).map(|pick| pick.tag.as_str())
    }

    #[test]
    fn an_8gb_laptop_gets_small_helpers_and_no_local_agent() {
        let advice = recommend(&machine(8.0, vec![]), &catalog());
        assert!(!advice.agent, "{advice:?}");
        assert!(tag(&advice, "helper").is_some() && tag(&advice, "embed").is_some(), "{advice:?}");
        assert!(advice.verdict.contains("as helpers"));
    }

    #[test]
    fn a_32gb_laptop_without_a_graphics_card_runs_a_mixture_of_experts_agent_slowly() {
        let iris = gpu_from("Intel(R) Iris(R) Xe Graphics", 2.0);
        assert!(!iris.dedicated);
        let advice = recommend(&machine(32.0, vec![iris]), &catalog());
        assert!(advice.agent, "{advice:?}");
        let agent = advice.picks.iter().find(|pick| pick.role == "agent").unwrap();
        assert_eq!(agent.runs_on, "cpu");
        assert!(advice.verdict.contains("slowly"));
    }

    #[test]
    fn a_24gb_card_runs_the_agent_on_the_card() {
        let advice = recommend(&machine(32.0, vec![gpu_from("NVIDIA GeForce RTX 4090", 24.0)]), &catalog());
        let agent = advice.picks.iter().find(|pick| pick.role == "agent").unwrap();
        assert_eq!(agent.runs_on, "gpu", "{advice:?}");
        assert!(agent.tokens_per_second >= 50);
    }

    fn builtin(advice: &BuiltinAdvice, id: &str) -> BuiltinPick {
        advice.models.iter().find(|pick| pick.id == id).cloned().unwrap_or_else(|| panic!("{id} missing: {advice:?}"))
    }

    #[test]
    fn the_built_in_engine_picks_a_quantization_that_fits() {
        let catalog = crate::registry::built_in_gguf();
        // This development machine: 32 GB, Iris Xe, no card of its own.
        let laptop = builtin_recommend(&machine(32.0, vec![gpu_from("Intel(R) Iris(R) Xe Graphics", 2.0)]), &catalog, Some(200.0));
        assert_eq!(laptop.backend, "cpu", "integrated graphics stay on the processor");
        let coder = builtin(&laptop, "qwen3-coder-30b-a3b");
        assert_eq!(coder.runs_on, "cpu");
        assert!(coder.fit != "no" && coder.tokens_per_second >= 8, "{coder:?}");
        let small = builtin(&laptop, "qwen3-4b-instruct-2507");
        assert!(matches!(small.fit.as_str(), "great" | "good"), "{small:?}");
        assert!(laptop.models.iter().filter(|pick| pick.recommended).count() >= 2);
        // An 8 GB laptop cannot hold the 30B model at all.
        let small_laptop = builtin_recommend(&machine(8.0, vec![]), &catalog, Some(200.0));
        assert_eq!(builtin(&small_laptop, "qwen3-coder-30b-a3b").fit, "no");
        assert!(small_laptop.models.iter().any(|pick| pick.recommended && pick.roles.iter().any(|r| r == "helper")));
        // A 24 GB NVIDIA card: CUDA, and the coder fully on the card.
        let desktop = builtin_recommend(&machine(32.0, vec![gpu_from("NVIDIA GeForce RTX 4090", 24.0)]), &catalog, Some(500.0));
        assert_eq!(desktop.backend, if cfg!(windows) { "cuda" } else { "vulkan" });
        let coder = builtin(&desktop, "qwen3-coder-30b-a3b");
        assert_eq!(coder.runs_on, "gpu", "{coder:?}");
        assert!(coder.recommended && coder.quant.starts_with('Q'), "{coder:?}");
        // A 12 GB card spills the dense 24B model partly into memory.
        let mid = builtin_recommend(&machine(32.0, vec![gpu_from("AMD Radeon RX 6700 XT", 12.0)]), &catalog, Some(500.0));
        assert_eq!(mid.backend, "vulkan");
        let devstral = builtin(&mid, "devstral-small-2507");
        assert!(matches!(devstral.runs_on.as_str(), "gpu" | "partial"), "{devstral:?}");
        // No disk space: nothing can be downloaded.
        let full = builtin_recommend(&machine(32.0, vec![]), &catalog, Some(0.5));
        assert!(full.models.iter().all(|pick| matches!(pick.fit.as_str(), "no" | "disk")));
        assert!(full.models.iter().any(|pick| pick.fit == "disk"));
    }

    #[test]
    fn the_context_fits_beside_the_weights() {
        let laptop = machine(32.0, vec![]);
        // 4B at Q4 (2.3 GB) with 147 KB a token at f16, about 78 KB at q8: the 64K cap applies.
        assert_eq!(plan_context(&laptop, 2.3, 147_456.0 * 8.5 / 16.0, 262_144), 65_536);
        assert_eq!(plan_context(&machine(16.0, vec![]), 2.3, 78_000.0, 262_144), 32_768, "32K on 16 GB");
        assert_eq!(plan_context(&laptop, 2.3, 78_000.0, 8_192), 8_192, "never more than trained");
        assert!(plan_context(&machine(8.0, vec![]), 3.5, 400_000.0, 131_072) >= 4096);
    }

    #[test]
    fn little_disk_space_is_called_out() {
        let mut hardware = machine(32.0, vec![]);
        hardware.disk_free_gb = Some(5.0);
        assert!(recommend(&hardware, &catalog()).verdict.contains("5 GB free"));
    }
}
