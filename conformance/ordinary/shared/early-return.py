def classify(n):
    if n < 0:
        return "neg"
    if n == 0:
        return "zero"
    return "pos"


async def run():
    return classify(-3)
