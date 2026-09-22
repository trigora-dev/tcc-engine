async def run():
    a = {"x": 1}
    b = a
    c = {"x": 1}
    return {"same": a is b, "other": a is c, "structural": a == c}
