async def run():
    x = 1

    def f():
        nonlocal x
        x = 2

    f()
    return x
