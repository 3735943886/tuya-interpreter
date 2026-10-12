"""Validate a release tag against Cargo and map SemVer to Python's version format."""
import argparse
import re
from pathlib import Path

# Restrict public releases to stable, alpha, beta and release-candidate versions.
PATTERN = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(alpha|beta|rc)\.([1-9]\d*))?")


def python_version(version):
    match = PATTERN.fullmatch(version)
    if not match:
        raise ValueError("Expected X.Y.Z or X.Y.Z-(alpha|beta|rc).N")
    major, minor, patch, stage, serial = match.groups()
    result = f"{major}.{minor}.{patch}"
    if stage:
        result += {"alpha": "a", "beta": "b", "rc": "rc"}[stage] + serial
    return result


def release_versions(cargo_version, tag=None):
    python = python_version(cargo_version)
    if tag is not None and tag != f"v{cargo_version}":
        raise ValueError(f"Release tag must be v{cargo_version}, got {tag}")
    return cargo_version, python


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag")
    parser.add_argument("--output", type=Path, help="Append GitHub Actions job outputs")
    args = parser.parse_args()
    manifest = Path(__file__).resolve().parents[1] / "rust" / "engine" / "Cargo.toml"
    package = manifest.read_text().split("[package]", 1)[1].split("\n[", 1)[0]
    name = re.search(r'^name\s*=\s*"([^"]+)"', package, re.MULTILINE).group(1)
    if name != "tuya-interpreter":
        parser.error("Cargo package must be named tuya-interpreter")
    version = re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE).group(1)
    try:
        cargo, python = release_versions(version, args.tag)
    except ValueError as error:
        parser.error(str(error))
    output = f"cargo_version={cargo}\npython_version={python}\n"
    print(output, end="")
    if args.output:
        with args.output.open("a", encoding="utf-8") as target:
            target.write(output)


if __name__ == "__main__":
    main()
