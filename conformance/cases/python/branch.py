from tcc_engine.primitives import program, effect, wait_for_event


@program
async def run():
    flag = await effect("generate", lambda: 1)
    if flag:
        approval = await wait_for_event("approved")
        return approval
    return 0
