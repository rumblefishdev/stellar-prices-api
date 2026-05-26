use extractors_core::{ExtractError, ExtractResult, SorobanEventRow, SwapExtractor};

pub struct PhoenixStablePoolExtractor;

impl SwapExtractor for PhoenixStablePoolExtractor {
    fn extract(&self, _rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        unimplemented!("Phoenix stable-pool extractor not yet implemented")
    }
}
