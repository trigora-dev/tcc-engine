def double(n):
    return n * 2


def square(n):
    return n * n


async def run():
    return square(double(3))
