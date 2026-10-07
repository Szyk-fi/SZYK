//! Pitch tables from the Rev2 guide, Appendix C. Frequencies are computed on
//! the UI thread; the callback only interpolates the resulting MIDI pitches.
pub const NAMES: &[&str] = &[
    "Equal temperament",
    "Harmonic series",
    "Carlos harmonic",
    "Meantone",
    "24 equal",
    "19 equal",
    "31 equal",
    "Pythagorean C",
    "Just A",
    "3-5 lattice A",
    "3-7 lattice A",
    "Other Music C",
    "Pelog / Slendro",
    "Yamaha major C",
    "Yamaha minor C",
    "Partch 43",
    "Arabic 12",
];
const CARLOS: [f32; 12] = [
    1.,
    17. / 16.,
    9. / 8.,
    19. / 16.,
    5. / 4.,
    21. / 16.,
    11. / 8.,
    3. / 2.,
    13. / 8.,
    27. / 16.,
    7. / 4.,
    15. / 8.,
];
const PARTCH: [f32; 43] = [
    1.,
    81. / 80.,
    33. / 32.,
    21. / 20.,
    16. / 15.,
    12. / 11.,
    11. / 10.,
    10. / 9.,
    9. / 8.,
    8. / 7.,
    7. / 6.,
    32. / 27.,
    6. / 5.,
    11. / 9.,
    5. / 4.,
    14. / 11.,
    9. / 7.,
    21. / 16.,
    4. / 3.,
    27. / 20.,
    11. / 8.,
    7. / 5.,
    10. / 7.,
    16. / 11.,
    40. / 27.,
    3. / 2.,
    32. / 21.,
    14. / 9.,
    11. / 7.,
    8. / 5.,
    18. / 11.,
    5. / 3.,
    27. / 16.,
    12. / 7.,
    7. / 4.,
    16. / 9.,
    9. / 5.,
    20. / 11.,
    11. / 6.,
    15. / 8.,
    40. / 21.,
    64. / 33.,
    160. / 81.,
];
fn periodic(n: i32, root: i32, hz: f32, ratios: &[f32]) -> f32 {
    let d = n - root;
    hz * 2.0f32.powi(d.div_euclid(ratios.len() as i32))
        * ratios[d.rem_euclid(ratios.len() as i32) as usize]
}
pub fn table(index: usize) -> [f32; 128] {
    std::array::from_fn(|n| {
        let n = n as i32;
        let hz = match index {
            1 if (36..=95).contains(&n) => 27.5 * (n - 34) as f32,
            1 | 2 => periodic(n, 69, 440., &CARLOS),
            3 => {
                let k = [0, 7, 2, -3, 4, -1, 6, 1, -4, 3, -2, 5][n.rem_euclid(12) as usize];
                let r = 5.0f32.powf(k as f32 / 4.);
                let ratio = r / 2.0f32.powf(r.log2().floor());
                260. * 2.0f32.powi((n - 60).div_euclid(12)) * ratio
            }
            4 | 5 | 6 => {
                let steps = [24., 19., 31.][index - 4];
                440. * 2.0f32.powf((n - 69) as f32 / steps)
            }
            7 => periodic(
                n,
                60,
                261.625,
                &[
                    1.,
                    256. / 243.,
                    9. / 8.,
                    32. / 27.,
                    81. / 64.,
                    4. / 3.,
                    729. / 512.,
                    3. / 2.,
                    128. / 81.,
                    27. / 16.,
                    16. / 9.,
                    243. / 128.,
                ],
            ),
            8 => periodic(
                n,
                69,
                440.,
                &[
                    1.,
                    16. / 15.,
                    9. / 8.,
                    6. / 5.,
                    5. / 4.,
                    4. / 3.,
                    7. / 5.,
                    3. / 2.,
                    8. / 5.,
                    5. / 3.,
                    9. / 5.,
                    15. / 8.,
                ],
            ),
            9 => periodic(
                n,
                69,
                440.,
                &[
                    1.,
                    16. / 15.,
                    10. / 9.,
                    6. / 5.,
                    5. / 4.,
                    4. / 3.,
                    64. / 45.,
                    3. / 2.,
                    8. / 5.,
                    5. / 3.,
                    16. / 9.,
                    15. / 8.,
                ],
            ),
            10 => periodic(
                n,
                69,
                440.,
                &[
                    1.,
                    9. / 8.,
                    8. / 7.,
                    7. / 6.,
                    9. / 7.,
                    21. / 16.,
                    4. / 3.,
                    3. / 2.,
                    32. / 21.,
                    12. / 7.,
                    7. / 4.,
                    63. / 32.,
                ],
            ),
            11 => periodic(
                n,
                60,
                261.625,
                &[
                    1.,
                    15. / 14.,
                    9. / 8.,
                    7. / 6.,
                    5. / 4.,
                    4. / 3.,
                    7. / 5.,
                    3. / 2.,
                    14. / 9.,
                    5. / 3.,
                    7. / 4.,
                    15. / 8.,
                ],
            ),
            12 => periodic(
                n,
                58,
                60.,
                &[
                    1.,
                    1.,
                    9. / 8.,
                    7. / 6.,
                    5. / 4.,
                    4. / 3.,
                    11. / 8.,
                    3. / 2.,
                    3. / 2.,
                    7. / 4.,
                    7. / 4.,
                    15. / 8.,
                ],
            ),
            13 => periodic(
                n,
                60,
                261.625,
                &[
                    1.,
                    16. / 15.,
                    9. / 8.,
                    6. / 5.,
                    5. / 4.,
                    4. / 3.,
                    45. / 32.,
                    3. / 2.,
                    8. / 5.,
                    5. / 3.,
                    16. / 9.,
                    15. / 8.,
                ],
            ),
            14 => periodic(
                n,
                60,
                261.625,
                &[
                    1.,
                    25. / 24.,
                    10. / 9.,
                    6. / 5.,
                    5. / 4.,
                    4. / 3.,
                    45. / 32.,
                    3. / 2.,
                    8. / 5.,
                    5. / 3.,
                    16. / 9.,
                    15. / 8.,
                ],
            ),
            15 => periodic(n, 67, 392., &PARTCH),
            16 => {
                let cents = [
                    0., 151., 204., 294., 355., 498., 649., 702., 853., 906., 996., 1057.,
                ];
                261.625
                    * 2.0f32.powf(
                        (n - 60).div_euclid(12) as f32
                            + cents[(n - 60).rem_euclid(12) as usize] / 1200.,
                    )
            }
            _ => 440. * 2.0f32.powf((n - 69) as f32 / 12.),
        };
        69. + 12. * (hz / 440.).log2()
    })
}
