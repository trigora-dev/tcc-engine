from tcc_engine.primitives import program
def double(n):
    return n * 2


def square(n):
    return n * n


@program
async def run():
    return square(double(3))
