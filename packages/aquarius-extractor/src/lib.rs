use extractors_core::{ExtractError, ExtractResult, SorobanEventRow, SwapExtractor};

pub struct AquariusPoolExtractor;

impl SwapExtractor for AquariusPoolExtractor {
    fn extract(&self, _rows: &[SorobanEventRow]) -> Result<ExtractResult, ExtractError> {
        unimplemented!("Aquarius pool extractor not yet implemented")
    }
}
