from tcc_engine.primitives import program, wait_for_event


def within_budget(amount, limit):
    return amount <= limit


@program
async def run():
    amount = 40
    limit = 50
    if not within_budget(amount, limit):
        return "rejected"
    decision = await wait_for_event("approval")
    if decision == "yes":
        return "approved"
    return "rejected"
