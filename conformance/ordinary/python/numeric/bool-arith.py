from tcc_engine.primitives import program
@program
async def run():
    return True + 1 + (False * 3)
