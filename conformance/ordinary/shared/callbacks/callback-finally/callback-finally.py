async def run():
    seen = 0

    def f():
        nonlocal seen
        try:
            raise Exception("x")
        finally:
            seen = 1

    try:
        f()
    except Exception:
        return seen
    return 0
