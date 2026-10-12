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


def project_version(root):
    """Ignore legacy tags; retain the already published first alpha as a baseline."""
    return get_version(
        root=str(root),
        version_scheme=version_scheme,
        scm={'git': {'describe_command': [
            'git', 'describe', '--dirty', '--tags', '--long',
            '--match', 'interpreter-v*', '--match', 'v0.1.0-alpha.1',
        ]}},
        parentdir_prefix_version='tuya-interpreter-',
    )


__version__ = project_version(Path(__file__).resolve().parent)
