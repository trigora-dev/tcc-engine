from tcc_engine.primitives import program
@program
async def run(value):
    return value * 3
