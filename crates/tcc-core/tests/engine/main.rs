mod lifecycle;
mod persistence;
mod resume;

use tcc_ir::EngineCaps;

fn caps() -> EngineCaps {
    EngineCaps::current()
}
