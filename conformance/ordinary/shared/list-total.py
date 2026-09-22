async def run():
    items = [
        {"price": 10, "quantity": 2},
        {"price": 4, "quantity": 3},
    ]
    limit = 20
    user = {"active": True}
    total = 0
    for item in items:
        total += item["price"] * item["quantity"]
    if total > limit and user["active"]:
        return total
    return 0
