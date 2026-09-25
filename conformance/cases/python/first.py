from tcc_engine.primitives import program, effect, wait_for_event


@program
async def run():
    result = await effect("generate", generate_something)
    approval = await wait_for_event("approved")
    return {"result": result, "approval": approval}
