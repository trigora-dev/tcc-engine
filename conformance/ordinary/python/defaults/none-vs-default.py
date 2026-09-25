from tcc_engine.primitives import program
def pick(value=7):
    if value is None:
        return 0
    return value


@program
async def run():
    omitted = pick()
    explicit = pick(None)
    return omitted + explicit
