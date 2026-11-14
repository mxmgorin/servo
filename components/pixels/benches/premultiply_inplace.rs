/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use criterion::*;
use pixels::Multiply;

fn create_data(number_of_pixels: usize) -> Vec<u8> {
    (0..number_of_pixels)
        .flat_map(|i| {
            [
                (i % 251) as u8,
                (i % 241) as u8,
                (i % 239) as u8,
                (i % 256) as u8,
            ]
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let data = create_data(1_000_000);

    let input = data.clone();
    c.bench_function("rgba8_premultiply_inplace", move |b| {
        b.iter_batched(
            || input.clone(),
            |mut data| pixels::rgba8_premultiply_inplace(&mut data),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("transform_inplace_premultiply", move |b| {
        b.iter_batched(
            || data.clone(),
            |mut data| pixels::transform_inplace(&mut data, Multiply::PreMultiply, false, false),
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
