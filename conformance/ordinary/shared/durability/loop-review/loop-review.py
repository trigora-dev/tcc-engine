from tcc_engine.primitives import program, wait_for_event


@program
async def run():
    items = [2, 5, 9]
    total = 0
    for n in items:
        total += n * 2
        if total > 10:
            decision = await wait_for_event("review")
            if decision == "stop":
                return total
    return total
