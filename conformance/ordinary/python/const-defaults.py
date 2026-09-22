async def run(a, b=10, mode="fast", enabled=True):
    total = a + b
    if enabled and mode == "fast":
        return total
    return 0
