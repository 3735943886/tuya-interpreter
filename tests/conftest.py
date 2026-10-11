import pathlib
import sys

HERE = pathlib.Path(__file__).parent
for p in (HERE, HERE / "golden", HERE.parent / "scripts"):   # `helpers`/`engine`, the `fixtures` loader, `standard`
    sys.path.insert(0, str(p))
