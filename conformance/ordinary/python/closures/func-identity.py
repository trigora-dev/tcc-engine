from tcc_engine.primitives import program
def double(n):
    return n * 2


@program
async def run():
    return 1 if double is double else 0
