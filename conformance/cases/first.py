from tcc_engine.primitives import effect, wait_for_event


async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
