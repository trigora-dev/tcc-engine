from tcc_engine.primitives import program
@program
async def run():
    def inner():
        raise Exception("nested")

    def outer():
        inner()

    try:
        outer()
        return 0
    except Exception:
        return 1
