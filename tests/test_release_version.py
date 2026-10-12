"""Both registries receive the same logical version; mismatched tags cannot publish."""
import pytest
from release_version import python_version, release_versions


@pytest.mark.parametrize("cargo,python", [
    ("0.1.0-alpha.1", "0.1.0a1"),
    ("0.1.0-alpha.12", "0.1.0a12"),
    ("0.1.0-beta.2", "0.1.0b2"),
    ("0.1.0-rc.1", "0.1.0rc1"),
    ("0.1.0", "0.1.0"),
])
def test_matching_tag_and_registry_versions(cargo, python):
    assert release_versions(cargo, f"interpreter-v{cargo}") == (cargo, python)


@pytest.mark.parametrize("version", ["0.1.0a1", "0.1.0-alpha", "0.1.0-alpha.0", "0.1.0+build", "01.1.0", "bad"])
def test_unsupported_versions_fail(version):
    with pytest.raises(ValueError):
        python_version(version)


def test_tag_must_match_cargo():
    with pytest.raises(ValueError, match="Release tag must be"):
        release_versions("0.1.0-alpha.1", "v0.2.0")


@pytest.mark.parametrize("tag", ["v0.1.0-alpha.1", "v0.4.0"])
def test_legacy_tags_cannot_trigger_new_releases(tag):
    with pytest.raises(ValueError, match="Release tag must be"):
        release_versions("0.1.0-alpha.1", tag)
