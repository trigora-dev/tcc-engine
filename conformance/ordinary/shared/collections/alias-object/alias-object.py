from tcc_engine.primitives import program
@program
async def run():
    a = {"x": 1}
    b = a
    b["x"] = 2
    return a["x"]
