"""Shared, build-time inventory for the site, application and offline notices."""

import argparse
import html
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def cargo_inventory(root):
    result = subprocess.run(
        [os.environ.get("CARGO", "cargo"), "metadata", "--locked", "--all-features", "--format-version", "1"],
        cwd=root, check=True, stdout=subprocess.PIPE, text=True,
    )
    metadata = json.loads(result.stdout)
    workspace = set(metadata["workspace_members"])
    direct = {
        dependency["pkg"]
        for node in metadata["resolve"]["nodes"] if node["id"] in workspace
        for dependency in node["deps"] if dependency["pkg"] not in workspace
    }
    packages = []
    for package in metadata["packages"]:
        if package["id"] in workspace:
            continue
        packages.append({
            "name": package["name"], "version": package["version"],
            "license": package["license"] or "Not declared",
            "url": package["repository"] or package["homepage"] or f'https://crates.io/crates/{package["name"]}/{package["version"]}',
            "direct": package["id"] in direct,
            "manifest": package["manifest_path"], "license_file": package["license_file"],
        })
    return metadata, sorted(packages, key=lambda item: (item["name"].lower(), item["version"]))


def notice_files(package):
    directory = Path(package["manifest"]).parent
    paths = set()
    if package["license_file"]:
        paths.add(directory / package["license_file"])
    # Crate distributions keep attribution alongside license files. Include nested
    # bundled-library notices without following symlinks outside the distribution.
    for path in directory.rglob("*"):
        if path.is_file() and not path.is_symlink() and path.suffix.lower() not in {".rs", ".c", ".h", ".js", ".py", ".json", ".toml"} and re.match(r"^(license|licence|copying|copyright|notice)([._-]|$)", path.name, re.I):
            paths.add(path)
    return sorted(paths)


def notice_overrides(root):
    index = root / "third-party/notices/index.json"
    return json.loads(index.read_text())


def supplemental_notices(root, entries):
    notices = []
    for entry in entries:
        path = root / "third-party/notices" / entry["file"]
        if path.parent != root / "third-party/notices":
            raise ValueError("Notice file must be inside third-party/notices")
        notices.append((entry["source"], path.read_text()))
    return notices


def asset_inventory(root):
    versions = {re.search(r"-v([0-9.]+)\.woff2$", path.name)[1] for path in (root / "frontend/fonts").glob("Monaspace*-v*.woff2")}
    if len(versions) != 1:
        raise ValueError("Expected one bundled Monaspace version")
    lucide = notice_overrides(root)["asset:Lucide"]
    return [{
        "name": "Monaspace (Argon, Neon, Xenon, Radon and Krypton)",
        "version": versions.pop(), "license": "SIL Open Font License 1.1",
        "url": "https://github.com/githubnext/monaspace", "direct": True,
        "notices": [("OFL.txt", (root / "frontend/fonts/OFL.txt").read_text())],
    }, {
        "name": "Lucide icons", "version": lucide[0]["source"].split("/")[-2],
        "license": "ISC", "url": "https://github.com/lucide-icons/lucide", "direct": True,
        "notices": supplemental_notices(root, lucide),
    }, {
        "name": "SQLite", "version": "bundled by libsqlite3-sys", "license": "Public domain",
        "url": "https://sqlite.org/copyright.html", "direct": True,
        "notices": [("Public-domain dedication", "The authors of SQLite have dedicated the code to the public domain.\nSee the bundled libsqlite3-sys notices and https://sqlite.org/copyright.html.")],
    }]


def link(label, url):
    label = html.escape(label)
    if url and url.startswith(("https://", "http://")):
        return f'<a href="{html.escape(url, quote=True)}" target="_blank" rel="noopener noreferrer">{label}</a>'
    return label


def package_html(package):
    title = html.escape(f'{package["name"]} · {package["version"]} · {package["license"]}')
    notices = "".join(f'<h4>{html.escape(name)}</h4><pre>{html.escape(text)}</pre>' for name, text in package["notices"])
    if not notices:
        notices = '<p>License text is not included in this crate distribution. See its upstream project.</p>'
    return f'<details class="about-package"><summary>{title}</summary><p>{link("Upstream project", package["url"])}</p>{notices}</details>'


def build_commit(root):
    supplied = os.environ.get("OPENWEBIDE_BUILD_COMMIT")
    if supplied:
        if supplied != "unknown" and not re.fullmatch(r"[0-9a-fA-F]{40,64}", supplied):
            raise ValueError("OPENWEBIDE_BUILD_COMMIT must be a full Git commit or 'unknown'")
        return supplied
    result = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True)
    if result.returncode:
        return "unknown (source archive)"
    modified = subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=root).returncode != 0
    return result.stdout.strip() + (" (modified)" if modified else "")


def render_app(root):
    metadata, packages = cargo_inventory(root)
    overrides = notice_overrides(root)
    watched = {root / "Cargo.lock", root / "LICENSE", root / "frontend/fonts/OFL.txt"}
    watched.update((root / "third-party/notices").glob("*"))
    watched.update((root / "frontend/fonts").glob("*"))
    for package in metadata["packages"]:
        watched.add(Path(package["manifest_path"]))
    for package in packages:
        paths = notice_files(package)
        watched.update(paths)
        package["notices"] = [(path.relative_to(Path(package["manifest"]).parent).as_posix(), path.read_text(errors="replace")) for path in paths]
        package["notices"].extend(supplemental_notices(root, overrides.get(package["name"] + "@" + package["version"], [])))
    version = next(package["version"] for package in metadata["packages"] if package["name"] == "openwebide-frontend")
    commit = build_commit(root)
    own = {"name": "Open WebIDE", "version": version, "license": "MIT", "url": "https://github.com/openwebide/openwebide", "notices": [("LICENSE", (root / "LICENSE").read_text())]}
    overview = (
        f'<dl class="about-build"><div><dt>Version</dt><dd>{html.escape(version)}</dd></div>'
        f'<div><dt>Commit</dt><dd><code>{html.escape(commit)}</code></dd></div></dl>'
        '<p>A self-hosted browser IDE for coding with local models.</p>'
    )
    software = (
        '<h2>Open-source software</h2><p>Third-party projects retain their own licenses. '
        'Expand a library to read its bundled or pinned upstream license texts and copyright notices.</p>'
        + package_html(own) + '<h3>Bundled assets</h3>'
        + ''.join(package_html(package) for package in asset_inventory(root))
        + f'<h3>Rust libraries ({len(packages)} crate versions)</h3>'
        '<p>This inventory follows Cargo.lock with all workspace features, including local and remote modes, '
        'platform-specific, build, test and benchmark dependencies. Each binary uses a subset.</p>'
        + ''.join(package_html(package) for package in packages)
    )
    if not os.environ.get("OPENWEBIDE_BUILD_COMMIT"):
        for name in ["HEAD", "index"]:
            result = subprocess.run(["git", "rev-parse", "--git-path", name], cwd=root, capture_output=True, text=True)
            if result.returncode == 0:
                watched.add(root / result.stdout.strip())
        branch = subprocess.run(["git", "symbolic-ref", "-q", "HEAD"], cwd=root, capture_output=True, text=True)
        if branch.returncode == 0:
            result = subprocess.run(["git", "rev-parse", "--git-path", branch.stdout.strip()], cwd=root, capture_output=True, text=True)
            if result.returncode == 0:
                watched.add(root / result.stdout.strip())
    return {"about": overview, "software": software}, watched


def render_page(fragment, stylesheets):
    styles = ''.join(f'<link rel="stylesheet" href="/{html.escape(path.name, quote=True)}">' for path in stylesheets)
    return ('<!doctype html><html lang="en" class="about-document"><head><meta charset="utf-8">'
            '<meta name="viewport" content="width=device-width,initial-scale=1">'
            '<title>About Open WebIDE</title>' + styles +
            '<script>document.documentElement.setAttribute("data-theme",matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light");</script>'
            '</head><body class="about-page"><main><h1>About Open WebIDE</h1>'
            '<div class="ui-segmented-control about-tabs" role="tablist" aria-label="About pages">'
            '<button class="ui-seg-btn active" id="about-tab-overview" role="tab" aria-selected="true" aria-controls="about-overview" tabindex="0">About</button>'
            '<button class="ui-seg-btn" id="about-tab-software" role="tab" aria-selected="false" aria-controls="about-software" tabindex="-1">Open-source software</button></div>'
            '<section class="about-content" id="about-overview" role="tabpanel" aria-labelledby="about-tab-overview" tabindex="0">' + fragment['about'] + '</section>'
            '<section class="about-content" id="about-software" role="tabpanel" aria-labelledby="about-tab-software" tabindex="0" hidden>' + fragment['software'] + '</section>'
            '<script>const tabs=[...document.querySelectorAll("[role=tab]")];'
            'function select(index,focus){tabs.forEach((tab,i)=>{const active=i===index;tab.setAttribute("aria-selected",String(active));tab.tabIndex=active?0:-1;tab.classList.toggle("active",active);document.getElementById(tab.getAttribute("aria-controls")).hidden=!active;});if(focus)tabs[index].focus();}'
            'tabs.forEach((tab,i)=>{tab.addEventListener("click",()=>select(i,false));tab.addEventListener("keydown",event=>{const key=event.key;if(!["ArrowLeft","ArrowRight","Home","End"].includes(key))return;event.preventDefault();select(key==="Home"?0:key==="End"?1:1-i,true);});});</script>' +
            '<p><a class="btn" href="/">Open WebIDE</a></p></main></body></html>')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fragment-output", type=Path)
    parser.add_argument("--page-output", type=Path)
    args = parser.parse_args()
    if args.fragment_output is None and args.page_output is None:
        parser.error("Provide an output path")
    fragment, watched = render_app(ROOT)
    if args.fragment_output:
        args.fragment_output.write_text(fragment["about"] + fragment["software"])
        args.fragment_output.with_name("about-overview.html").write_text(fragment["about"])
        args.fragment_output.with_name("about-software.html").write_text(fragment["software"])
        for path in sorted(watched):
            print(f"cargo::rerun-if-changed={path}")
    if args.page_output:
        stylesheets = sorted(args.page_output.parent.glob("*.css"))
        if not stylesheets:
            raise ValueError("The About page requires the built frontend stylesheet")
        args.page_output.write_text(render_page(fragment, stylesheets))


if __name__ == "__main__":
    main()
