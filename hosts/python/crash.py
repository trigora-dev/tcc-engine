# Copyright (c) 2026 Trigora, Inc.
# SPDX-License-Identifier: BUSL-1.1
# See LICENSE for full terms.

from __future__ import annotations

import os
import signal

_counts: dict[str, int] = {}


def maybe_crash(hook: str, detail: str | None = None) -> None:
    want = os.environ.get("TCC_CRASH_AT")
    if not want:
        return
    n = _bump(hook)
    if want == hook or want == f"{hook}:{n}" or (detail is not None and want == f"{hook}:{detail}"):
        os.kill(os.getpid(), signal.SIGKILL)


def _bump(hook: str) -> int:
    n = _counts.get(hook, 0) + 1
    _counts[hook] = n
    return n
