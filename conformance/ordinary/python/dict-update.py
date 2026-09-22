async def run():
    item = {"price": 4, "quantity": 3}
    extra = item
    extra["price"] = 5
    return item["price"] * item["quantity"]
