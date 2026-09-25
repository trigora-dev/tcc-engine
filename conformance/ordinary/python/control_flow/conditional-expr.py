from tcc_engine.primitives import program
@program
async def run():
    n = -2
    return "pos" if n > 0 else "neg" if n < 0 else "zero"
