"""VCS versions, including commits after a numbered development tag."""
from pathlib import Path

from setuptools_scm import get_version
from setuptools_scm.version import guess_next_dev_version


def version_scheme(version):
    if version.exact:
        return version.format_with('{tag}')
    tag = version.tag
    if tag.dev is not None:
        # v0.4.0.dev1 + three commits -> 0.4.0.dev4, before the final 0.4.0.
        base = str(tag).split('.dev', 1)[0]
        return f'{base}.dev{tag.dev + max(version.distance, 1)}'
    return guess_next_dev_version(version)


__version__ = get_version(
    root=str(Path(__file__).resolve().parent),
    version_scheme=version_scheme,
    parentdir_prefix_version='tuya2ildevice-',
)
