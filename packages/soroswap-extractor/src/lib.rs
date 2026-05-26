use extractors_core::{ExtractError, ExtractResult, SorobanEventRow, SwapExtractor};

pub struct SoroswapPairExtractor;

impl SwapExtractor for SoroswapPairExtractor {
    fn extract(&self, _rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        unimplemented!("Soroswap pair extractor not yet implemented")
    }
}
