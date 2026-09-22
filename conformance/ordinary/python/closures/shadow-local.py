async def run():
    x = 1

    def f():
        x = 2
        return x

    return f() + x * 10
