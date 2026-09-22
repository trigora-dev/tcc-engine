from tcc_engine.primitives import wait_for_event


async def run():
    multiplier = 3

    def transform(value):
        return value * multiplier

    value = await wait_for_event("go")
    return transform(value)
