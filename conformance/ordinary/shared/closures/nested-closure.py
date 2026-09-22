async def run():
    a = 2

    def f():
        b = 3

        def g():
            return a + b

        return g()

    return f()
