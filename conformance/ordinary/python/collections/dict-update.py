from tcc_engine.primitives import program
@program
async def run():
    item = {"price": 4, "quantity": 3}
    extra = item
    extra["price"] = 5
    return item["price"] * item["quantity"]
