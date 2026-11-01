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

fn gpu_from(name: &str, vram_gb: f32) -> Gpu {
    let dedicated = !integrated(name) && vram_gb >= 2.0;
    Gpu { name: name.to_string(), vram_gb: if dedicated { vram_gb } else { 0.0 }, dedicated }
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

    #[test]
    fn little_disk_space_is_called_out() {
        let mut hardware = machine(32.0, vec![]);
        hardware.disk_free_gb = Some(5.0);
        assert!(recommend(&hardware, &catalog()).verdict.contains("5 GB free"));
    }
}
