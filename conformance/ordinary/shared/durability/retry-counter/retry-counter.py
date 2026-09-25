from tcc_engine.primitives import program, effect


def next_attempt(attempt):
    return attempt + 1


@program
async def run():
    attempt = 0
    accepted = False
    while not accepted and attempt < 3:
        attempt = next_attempt(attempt)
        status = await effect("try-send", send)
        if status == "ok":
            accepted = True
    return attempt
