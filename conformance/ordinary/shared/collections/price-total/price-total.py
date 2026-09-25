from tcc_engine.primitives import program
@program
async def run():
    price = 10
    quantity = 3
    tax = 2
    limit = 20
    user = {"active": True}
    subtotal = price * quantity
    total = subtotal + tax
    if total > limit and user["active"]:
        return total
    return 0
