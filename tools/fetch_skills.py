"""Fetches the skills Neru ships with into app/src-tauri/skills/, with their licenses.

Each skill is a folder with a SKILL.md (Anthropic's Agent Skills format) plus the reference
documents it points to. Scripts, evals, fixtures and images are left out: Neru's agent reads and
follows the instructions itself. Every skill folder keeps the license of its source, and
skills/NOTICE.md credits the authors.

Needs the GitHub CLI (gh) signed in. Run from D:\\Neru:  python tools/fetch_skills.py
"""

import base64
import json
import shutil
import subprocess
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "app" / "src-tauri" / "skills"
TEXT = (".md", ".json", ".txt")
SKIP_DIRS = {"evals", "evals-extra", "fixtures", "assets", "scripts", "agents", "eval-viewer", "examples", "degraded", ".claude-plugin", "commands"}

# (repository, branch, folder inside the repo, name in Neru, license id, credit)
SOURCES = [
    # Design
    ("anthropics/skills", "main", "skills/frontend-design", "frontend-design", "Apache-2.0", "Anthropic"),
    ("pbakaus/impeccable", "main", ".claude/skills/impeccable", "impeccable", "Apache-2.0", "Paul Bakaus (Impeccable)"),
    # Engineering method (Superpowers)
    ("obra/superpowers", "main", "skills/brainstorming", "brainstorming", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/writing-plans", "writing-plans", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/executing-plans", "executing-plans", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/test-driven-development", "test-driven-development", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/systematic-debugging", "systematic-debugging", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/verification-before-completion", "verification-before-completion", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/requesting-code-review", "requesting-code-review", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/receiving-code-review", "receiving-code-review", "MIT", "Jesse Vincent (Superpowers)"),
    ("obra/superpowers", "main", "skills/finishing-a-development-branch", "finishing-a-development-branch", "MIT", "Jesse Vincent (Superpowers)"),
    # Security (Trail of Bits)
    ("trailofbits/skills", "main", "plugins/sharp-edges/skills/sharp-edges", "sharp-edges", "CC-BY-SA-4.0", "Trail of Bits"),
    ("trailofbits/skills", "main", "plugins/differential-review/skills/differential-review", "differential-review", "CC-BY-SA-4.0", "Trail of Bits"),
    ("trailofbits/skills", "main", "plugins/supply-chain-risk-auditor/skills/supply-chain-risk-auditor", "supply-chain-risk-auditor", "CC-BY-SA-4.0", "Trail of Bits"),
    ("trailofbits/skills", "main", "plugins/property-based-testing/skills/property-based-testing", "property-based-testing", "CC-BY-SA-4.0", "Trail of Bits"),
    ("trailofbits/skills", "main", "plugins/audit-context-building/skills/audit-context-building", "audit-context-building", "CC-BY-SA-4.0", "Trail of Bits"),
    # Building agents
    ("anthropics/skills", "main", "skills/mcp-builder", "mcp-builder", "Apache-2.0", "Anthropic"),
]

LICENSE_FILES = {
    "anthropics/skills": None,  # each skill folder carries LICENSE.txt
    "obra/superpowers": "LICENSE",
    "trailofbits/skills": "LICENSE",
    "pbakaus/impeccable": "LICENSE",
}


def gh(path: str):
    return json.loads(subprocess.run(["gh", "api", path], check=True, capture_output=True, text=True, encoding="utf-8").stdout)


def blob(repo: str, sha: str) -> bytes:
    return base64.b64decode(gh(f"repos/{repo}/git/blobs/{sha}")["content"])


def main():
    trees = {}
    licenses = {}
    if OUT.exists():
        for child in OUT.iterdir():
            if child.is_dir() and not child.name.startswith("neru-"):
                shutil.rmtree(child)
    OUT.mkdir(parents=True, exist_ok=True)
    for repo, branch, folder, name, license_id, credit in SOURCES:
        if repo not in trees:
            trees[repo] = gh(f"repos/{repo}/git/trees/{branch}?recursive=1")["tree"]
        files = [item for item in trees[repo] if item["type"] == "blob" and item["path"].startswith(folder + "/")]
        if not any(item["path"] == f"{folder}/SKILL.md" for item in files):
            raise SystemExit(f"{repo}:{folder} has no SKILL.md")
        target = OUT / name
        for item in files:
            relative = Path(item["path"][len(folder) + 1:])
            if any(part in SKIP_DIRS for part in relative.parts[:-1]) or (relative.suffix not in TEXT and relative.name != "LICENSE.txt"):
                continue
            destination = target / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(blob(repo, item["sha"]))
        license_path = LICENSE_FILES[repo]
        if license_path and not (target / "LICENSE.txt").exists():
            if repo not in licenses:
                entry = next(item for item in trees[repo] if item["path"] == license_path)
                licenses[repo] = blob(repo, entry["sha"])
            (target / "LICENSE.txt").write_bytes(licenses[repo])
        (target / "SOURCE.txt").write_text(f"From https://github.com/{repo}/tree/{branch}/{folder}\nBy {credit}. License: {license_id} (see LICENSE.txt).\n", encoding="utf-8")
        print(f"{name:34} {license_id:14} {sum(1 for _ in target.rglob('*') if _.is_file())} files")
    notice = ["# Skills bundled with Neru", "", "These skills are included unchanged from their authors, under their own licenses. Each folder has its LICENSE.txt and SOURCE.txt.", ""]
    for repo, _branch, folder, name, license_id, credit in SOURCES:
        notice.append(f"- **{name}** from [{repo}](https://github.com/{repo}) by {credit}, {license_id}")
    notice += ["", "Skills whose names start with `neru-` were written for Neru and are MIT licensed like the app."]
    (OUT / "NOTICE.md").write_text("\n".join(notice) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
