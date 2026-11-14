/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use euclid::default::Size2D;
use pixels::{
    Multiply, flip_y_rgba8_image_inplace, multiply_u8_color, rgba8_premultiply_inplace,
    transform_inplace,
};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];

const COLORS: [[u8; 4]; 4] = [RED, GREEN, BLUE, YELLOW];

fn create_rgba8_image(number_of_pixels: usize) -> Vec<u8> {
    (0..number_of_pixels)
        .map(|i| COLORS[i % 4])
        .flatten()
        .collect()
}

#[test]
fn test_flip_y_rgba8_image_inplace() {
    // | R G | B Y | -> | B Y | R G |
    let mut image2x2 = create_rgba8_image(4);

    flip_y_rgba8_image_inplace(Size2D::new(2, 2), &mut image2x2);

    assert_eq!(
        &image2x2[0..4],
        &BLUE,
        "Expected blue color at [0, 0] (image2x2)"
    );
    assert_eq!(
        &image2x2[12..16],
        &GREEN,
        "Expected green color at [1, 1] (image2x2)"
    );

    // | R G B | Y R G | B Y R | -> | B Y R | Y R G | R G B |
    let mut image3x3 = create_rgba8_image(9);

    flip_y_rgba8_image_inplace(Size2D::new(3, 3), &mut image3x3);

    assert_eq!(
        &image3x3[0..4],
        &BLUE,
        "Expected blue color at [0, 0] (image3x3)"
    );
    assert_eq!(
        &image3x3[16..20],
        &RED,
        "Expected red color at [1, 1] (image3x3)"
    );
    assert_eq!(
        &image3x3[32..36],
        &BLUE,
        "Expected blue color at [2, 2] (image3x3)"
    );
}

#[test]
fn test_premultiply_matches_multiply_u8_color() {
    // Every color under every alpha, with distinct channels.
    let pixels: Vec<u8> = (0..=u8::MAX)
        .flat_map(|alpha| (0..=u8::MAX).map(move |color| [color, !color, color / 3, alpha]))
        .flatten()
        .collect();
    let expected: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| {
            [
                multiply_u8_color(r, a),
                multiply_u8_color(g, a),
                multiply_u8_color(b, a),
                a,
            ]
        })
        .collect();

    let mut premultiplied = pixels.clone();
    assert!(!rgba8_premultiply_inplace(&mut premultiplied));
    assert_eq!(premultiplied, expected, "rgba8_premultiply_inplace");

    let mut opaque: Vec<u8> = (0..=u8::MAX)
        .flat_map(|color| [color, !color, color / 3, u8::MAX])
        .collect();
    assert!(rgba8_premultiply_inplace(&mut opaque), "opaque pixels");

    let mut premultiplied = pixels.clone();
    transform_inplace(&mut premultiplied, Multiply::PreMultiply, false, false);
    assert_eq!(premultiplied, expected, "transform_inplace");
}
