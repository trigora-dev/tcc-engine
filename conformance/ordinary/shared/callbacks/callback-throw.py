async def run():
    try:

        def f():
            raise Exception("boom")

        f()
        return 0
    except Exception:
        return 1
