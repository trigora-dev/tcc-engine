from tcc_engine.primitives import program
def update(x):
    x["count"] += 1


@program
async def run():
    state = {"count": 0}
    update(state)
    return state["count"]
