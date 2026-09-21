from tcc_engine.primitives import effect, wait_for_event


async def run():
    flag = await effect("generate", lambda: 1)
    if flag:
        approval = await wait_for_event("approved")
        return approval
    return 0
