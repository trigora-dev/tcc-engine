from tcc_engine.primitives import program
@program
async def run():
    pair = [4, 5]
    left, right = pair
    return left + right
