from tcc_engine.primitives import program, invoke


@program
async def run():
    value = await invoke("child", 2)
    return value + 1
