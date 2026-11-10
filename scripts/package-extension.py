#!/usr/bin/env python3
"""Package the VS Code extension as a .vsix, with nothing but the standard library.

A .vsix is a zip archive with a fixed layout:

    [Content_Types].xml        what each file extension is
    extension.vsixmanifest     the name, version and engine, for the installer
    extension/                 the extension itself (package.json, src/, media/)

The usual tool for this is `vsce`, which needs Node and a package download.
This script writes the same archive directly, so a release can be built on any
machine that has Python, and the result installs with:

    code --install-extension target/extension/cohors-<version>.vsix

Usage:
    scripts/package-extension.py [--out DIR]

The development folder (`extension/dev`) and anything named in
`extension/.vscodeignore` are left out.
"""

import argparse
import fnmatch
import json
import os
import sys
import zipfile
from xml.sax.saxutils import escape

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "extension")

# What each kind of file is, as the archive's index declares it.
CONTENT_TYPES = {
    ".json": "application/json",
    ".vsixmanifest": "text/xml",
    ".js": "application/javascript",
    ".css": "text/css",
    ".svg": "image/svg+xml",
    ".png": "image/png",
    ".ttf": "application/octet-stream",
    ".md": "text/markdown",
    ".txt": "text/plain",
}


def ignored_patterns():
    """The patterns in .vscodeignore, plus the folders that are never shipped."""
    patterns = ["dev/**", "node_modules/**", ".vscodeignore", "*.vsix"]
    path = os.path.join(SOURCE, ".vscodeignore")
    if os.path.exists(path):
        for line in open(path, encoding="utf-8"):
            line = line.strip()
            if line and not line.startswith("#"):
                patterns.append(line)
    return patterns


def is_ignored(relative, patterns):
    """Whether `relative` (a path inside extension/, with forward slashes) is left out."""
    for pattern in patterns:
        pattern = pattern.lstrip("/")
        if fnmatch.fnmatch(relative, pattern):
            return True
        # `dev/**` and plain `dev` both mean the whole folder.
        folder = pattern[:-3] if pattern.endswith("/**") else pattern.rstrip("/")
        if relative == folder or relative.startswith(folder + "/"):
            return True
    return False


def files_to_ship():
    patterns = ignored_patterns()
    shipped = []
    for directory, folders, names in os.walk(SOURCE):
        folders.sort()
        for name in sorted(names):
            full = os.path.join(directory, name)
            relative = os.path.relpath(full, SOURCE).replace(os.sep, "/")
            if not is_ignored(relative, patterns):
                shipped.append((full, relative))
    return shipped


def manifest(package, has_readme):
    """extension.vsixmanifest: what the installer reads before unpacking."""
    engine = package.get("engines", {}).get("vscode", "*")
    categories = ",".join(package.get("categories", ["Other"]))
    tags = ",".join(package.get("keywords", []))
    assets = [
        '<Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />',
        '<Asset Type="Microsoft.VisualStudio.Services.Content.License" Path="extension/LICENSE.txt" Addressable="true" />',
    ]
    if has_readme:
        assets.append(
            '<Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/README.md" Addressable="true" />'
        )
    return f"""<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="{escape(package["name"])}" Version="{escape(package["version"])}" Publisher="{escape(package["publisher"])}" />
    <DisplayName>{escape(package.get("displayName", package["name"]))}</DisplayName>
    <Description xml:space="preserve">{escape(package.get("description", ""))}</Description>
    <Tags>{escape(tags)}</Tags>
    <Categories>{escape(categories)}</Categories>
    <GalleryFlags>Public</GalleryFlags>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="{escape(engine)}" />
      <Property Id="Microsoft.VisualStudio.Code.ExtensionDependencies" Value="" />
      <Property Id="Microsoft.VisualStudio.Code.ExtensionPack" Value="" />
      <Property Id="Microsoft.VisualStudio.Code.ExtensionKind" Value="workspace" />
      <Property Id="Microsoft.VisualStudio.Code.LocalizedLanguages" Value="" />
      <Property Id="Microsoft.VisualStudio.Services.GitHubFlavoredMarkdown" Value="true" />
    </Properties>
    <License>extension/LICENSE.txt</License>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code" />
  </Installation>
  <Dependencies />
  <Assets>
    {chr(10).join("    " + asset for asset in assets).strip()}
  </Assets>
</PackageManifest>
"""


def content_types(extensions):
    """[Content_Types].xml: one line per file extension in the archive."""
    lines = [
        f'<Default Extension="{extension}" ContentType="{CONTENT_TYPES.get(extension, "application/octet-stream")}" />'
        for extension in sorted(extensions)
    ]
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        + "".join(lines)
        + "</Types>\n"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--out",
        default=os.path.join(ROOT, "target", "extension"),
        help="folder for the .vsix (default: target/extension/)",
    )
    args = parser.parse_args()

    package = json.load(open(os.path.join(SOURCE, "package.json"), encoding="utf-8"))
    for field in ("name", "version", "publisher", "main"):
        if not package.get(field):
            sys.exit(f"extension/package.json has no {field!r}")
    if not os.path.exists(os.path.join(SOURCE, package["main"])):
        sys.exit(f"extension/package.json points at {package['main']}, which is not there")

    shipped = files_to_ship()
    names = {relative for _, relative in shipped}
    if "LICENSE" not in names and "LICENSE.txt" not in names:
        sys.exit("extension/ has no LICENSE")

    os.makedirs(args.out, exist_ok=True)
    target = os.path.join(args.out, f"{package['name']}-{package['version']}.vsix")
    extensions = {".vsixmanifest", ".json"}
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED) as archive:
        for full, relative in shipped:
            # The installer wants every file to have an extension it knows.
            stored = "LICENSE.txt" if relative == "LICENSE" else relative
            extension = os.path.splitext(stored)[1].lower()
            if not extension:
                sys.exit(f"extension/{relative} has no file extension; the installer cannot type it")
            extensions.add(extension)
            archive.write(full, "extension/" + stored)
        archive.writestr("extension.vsixmanifest", manifest(package, "README.md" in names))
        archive.writestr("[Content_Types].xml", content_types(extensions))

    size = os.path.getsize(target) / 1024
    print(f"{os.path.relpath(target, ROOT)}  ({len(shipped)} files, {size:.0f} KB)")
    print(f"install it with:  code --install-extension {os.path.relpath(target, ROOT)}")


if __name__ == "__main__":
    main()
