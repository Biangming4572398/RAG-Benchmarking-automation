use polars::prelude::*;

fn load_benchmarks(url: &str) -> PolarsResult<DataFrame> {
    //loading a benchmark from huggingface
    let path = PlRefPath::new(url);
    let df = LazyFrame::scan_parquet(path, ScanArgsParquet::default())?.collect()?;
    Ok(df)
}

#[cfg(test)]
mod test {}
