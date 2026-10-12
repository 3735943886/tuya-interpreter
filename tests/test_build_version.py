"""Development tags advance without rewriting tags; exact release tags stay exact."""
import runpy
from datetime import datetime, timezone
from pathlib import Path

import pytest
from setuptools_scm import Configuration
from setuptools_scm.version import meta

scheme = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'hatch_version.py'))['version_scheme']


@pytest.mark.parametrize('tag,distance,dirty,expected', [
    ('0.4.0.dev1', 0, False, '0.4.0.dev1'),
    ('0.4.0.dev1', 3, False, '0.4.0.dev4'),
    ('0.4.0.dev1', 0, True, '0.4.0.dev2'),
    ('0.4.0.dev0', 3, False, '0.4.0.dev3'),
    ('0.4.0', 0, False, '0.4.0'),
    ('0.1.0-alpha.1', 0, False, '0.1.0a1'),
    ('0.4.0', 3, False, '0.4.1.dev3'),
])
def test_version_scheme(tag, distance, dirty, expected):
    version = meta(tag, config=Configuration(), distance=distance, dirty=dirty,
                   time=datetime(2026, 1, 1, tzinfo=timezone.utc))
    assert scheme(version) == expected
