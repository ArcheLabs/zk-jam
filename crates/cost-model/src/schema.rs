pub const SCHEMA_VERSION: &str = "pvm-openvm-cost-model-v1";
pub const TRANSLATION_VERSION: u32 = 1;

pub fn write_schema(output: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(
        output,
        include_str!("../../../benchmarks/schema/pvm-openvm-cost-model-v1.schema.json"),
    )
}
