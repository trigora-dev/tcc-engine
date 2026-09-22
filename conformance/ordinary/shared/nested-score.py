def score(a, b, c):
    return (a + b) * c - a / 2


async def run():
    return score(4, 2, 3)
