use ogame_core::market::CandlestickScale;

pub fn format_timestamp(base_timestamp: i32, offset: f64, scale: CandlestickScale) -> String {
    let timestamp = base_timestamp as i64 + (offset as i64 * scale.to_seconds() as i64);
    let dt = chrono::DateTime::from_timestamp(timestamp, 0).unwrap();

    match scale {
        CandlestickScale::Minute1 | CandlestickScale::Minute30 => dt.format("%H:%M").to_string(),
        CandlestickScale::Hour1 | CandlestickScale::Hour4 => dt.format("%m-%d %H:%M").to_string(),
        CandlestickScale::Day1 => dt.format("%Y-%m-%d").to_string(),
    }
}
