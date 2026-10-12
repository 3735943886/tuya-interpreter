"""Bundle the C ABI library and portable rule pack into platform-specific wheels."""
import os
import subprocess
import sys
import sysconfig
from pathlib import Path

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version, build_data):
        if self.target_name != 'wheel':
            return
        root = Path(self.root)
        manifest = root / 'rust/Cargo.toml'
        target = Path(os.environ.get('CARGO_TARGET_DIR', root / 'rust/target'))
        if not target.is_absolute():
            target = root / target
        name = 'tuya_interpreter.dll' if sys.platform == 'win32' else 'libtuya_interpreter.dylib' if sys.platform == 'darwin' else 'libtuya_interpreter.so'
        supplied = os.environ.get('TUYA_ENGINE_BUNDLED_LIBRARY')
        triple = os.environ.get('TUYA_ENGINE_TARGET') or os.environ.get('CARGO_BUILD_TARGET')
        if supplied:
            library = Path(supplied)
            if not library.is_absolute():
                library = root / library
        else:
            command = ['cargo', 'build', '--release', '--manifest-path', str(manifest)]
            if triple:
                command.extend(['--target', triple])
            subprocess.run(command, check=True)
            library = target / triple / 'release' / name if triple else target / 'release' / name
        if not library.is_file():
            raise FileNotFoundError(f'Native library was not built: {library}')
        build_data['pure_python'] = False
        platform = sysconfig.get_platform().replace('-', '_').replace('.', '_')
        build_data['tag'] = 'py3-none-' + platform
        build_data['force_include'][str(library)] = 'tuya_interpreter/' + name
        # Bundle the official rule layers and their manifest together.
        for layer in sorted((root / 'rules' / 'builtin').glob('*.json')):
            build_data['force_include'][str(layer)] = 'tuya_interpreter/rules/builtin/' + layer.name
