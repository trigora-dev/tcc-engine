async def run():
    x = 1

    def read():
        return x

    x = 2
    return read()
