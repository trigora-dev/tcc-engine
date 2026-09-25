from tcc_engine.primitives import program
@program
async def run():
    xs = [10, 20, 30]
    return xs[-1] + xs[-3]
