from tcc_engine.primitives import program
@program
async def run():
    xs = [1, 2]
    xs.append(3)
    return len(xs)
