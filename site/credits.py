"""Generate acknowledgements from Cargo and installed site-build metadata."""

import re
import sys
from pathlib import Path
from importlib.metadata import distribution

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from tools.open_source import cargo_inventory, asset_inventory

from packaging.requirements import Requirement
from packaging.utils import canonicalize_name


def cell(value):
    return re.sub(r"\s+", " ", str(value or "Not declared")).replace("|", "&#124;").replace("<", "&lt;").replace(">", "&gt;")


def table(packages):
    rows = ["| Library | Version | Declared license |", "| --- | --- | --- |"]
    for name, version, license_name, url in sorted(packages, key=lambda item: (item[0].lower(), item[1])):
        label = cell(name)
        if url and url.startswith(("https://", "http://")):
            label = f"[{label}](<{url}>)"
        rows.append(f"| {label} | {cell(version)} | {cell(license_name)} |")
    return "\n".join(rows)


def python_packages(requirements):
    pending = [Requirement(line).name for line in requirements.read_text().splitlines() if line.strip() and not line.startswith("#")]
    found = {}
    while pending:
        name = canonicalize_name(pending.pop())
        if name in found:
            continue
        package = distribution(name)
        meta = package.metadata
        urls = [entry.split(", ", 1) for entry in meta.get_all("Project-URL", [])]
        source = next((url for label, url in urls if "source" in label.lower()), meta.get("Home-page"))
        found[name] = (meta["Name"], package.version, meta.get("License-Expression") or meta.get("License") or "; ".join(c.split(" :: ")[-1] for c in meta.get_all("Classifier", []) if c.startswith("License ::")), source or f"https://pypi.org/project/{name}/{package.version}/")
        for entry in meta.get_all("Requires-Dist", []):
            dependency = Requirement(entry)
            if dependency.marker is None or dependency.marker.evaluate({"extra": ""}):
                pending.append(dependency.name)
    return table(found.values())


def render(root):
    _, inventory = cargo_inventory(root)
    packages = [(package["direct"], (package["name"], package["version"], package["license"], package["url"])) for package in inventory]
    direct_rows = [row for is_direct, row in packages if is_direct]
    transitive_rows = [row for is_direct, row in packages if not is_direct]
    diagrams = (root / "site/assets/diagrams.js").read_text()
    mermaid = re.search(r"mermaid@([0-9.]+)/", diagrams)
    elk = re.search(r"@mermaid-js/layout-elk@([0-9.]+)/", diagrams)
    if mermaid is None or elk is None:
        raise ValueError("Missing pinned Mermaid or ELK layout version")
    return (
        f"\n## Rust libraries\n\nThis build credits **{len(packages)} third-party crate versions**, "
        "resolved from `Cargo.lock` with all workspace features. This includes both workspace modes, "
        "platform-specific, build, development and benchmark dependencies; individual binaries use a subset. "
        "Locally patched crates are included using their vendored manifest metadata.\n\n"
        "### Direct dependencies\n\n" + table(direct_rows) +
        f'\n\n<details markdown="1">\n<summary>Transitive dependencies ({len(transitive_rows)} crate versions)</summary>\n\n' +
        table(transitive_rows) + "\n\n</details>\n\n## Bundled assets\n\n" +
        table([(asset["name"], asset["version"], asset["license"], asset["url"]) for asset in asset_inventory(root)]) +
        "\n\n## Website build libraries\n\n"
        "These packages build the documentation site. Versions reflect the environment used for this build.\n\n" +
        python_packages(root / "site/requirements.txt") + "\n\n## Website browser library\n\n" +
        table([
            ("Mermaid", mermaid[1], "MIT", "https://github.com/mermaid-js/mermaid"),
            ("Mermaid ELK layout", elk[1], "MIT", "https://github.com/mermaid-js/mermaid/tree/develop/packages/mermaid-layout-elk"),
        ]) +
        "\n\nMermaid renders the architecture diagrams; its ELK layout uses elkjs (EPL-2.0). "
        "Both distributions include bundled dependencies. "
        "See its upstream distribution for third-party notices.\n"
    )
