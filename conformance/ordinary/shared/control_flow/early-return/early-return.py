from tcc_engine.primitives import program
def classify(n):
    if n < 0:
        return "neg"
    if n == 0:
        return "zero"
    return "pos"


@program
async def run():
    return classify(-3)
