from tcc_engine.primitives import invoke


async def run():
    value = await invoke("child", 2)
    return value + 1
