async def run():
    def f():
        try:
            raise Exception("inner")
        except Exception:
            return 1

    try:
        return f()
    except Exception:
        return 0
