from crash import maybe_crash
from tcc_engine.host import (
    encode_value,
    map_effects,
    resume_execution as _resume_execution,
    start_execution as _start_execution,
)

FakeEffects = dict


def start_execution(**kwargs):
    kwargs.setdefault("crash", maybe_crash)
    return _start_execution(**kwargs)


def resume_execution(**kwargs):
    kwargs.setdefault("crash", maybe_crash)
    return _resume_execution(**kwargs)
