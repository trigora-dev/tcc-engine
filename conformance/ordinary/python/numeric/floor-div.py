from tcc_engine.primitives import program
@program
async def run():
    return ((-7) // 3) * 100 + (7.5 // 2) * 10 + ((-7.5) // 2)
