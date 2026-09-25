from tcc_engine.primitives import program
@program
async def run():
    funcs = []
    for x in [1, 2, 3]:
        funcs.append(lambda: x)
    return funcs[0]()
