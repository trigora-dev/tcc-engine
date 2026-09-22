def update(x):
    x["count"] += 1


async def run():
    state = {"count": 0}
    update(state)
    return state["count"]
