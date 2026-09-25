from tcc_engine.primitives import program
def score(a, b, c):
    return (a + b) * c - a / 2


@program
async def run():
    return score(4, 2, 3)
