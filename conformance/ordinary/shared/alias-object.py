async def run():
    a = {"x": 1}
    b = a
    b["x"] = 2
    return a["x"]
