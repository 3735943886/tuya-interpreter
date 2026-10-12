"""Override blocks: the error a refused block raises and the engine's merge."""
from .native import call


class OverrideError(ValueError):
    pass


def merge_all(mappings):
    """Merge override mappings in order (objects recursively, other values replace)."""
    return call('merge', mappings=list(mappings))
