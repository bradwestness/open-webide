#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub position: f64,
    pub hashes: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timeline {
    pub clusters: Vec<Cluster>,
    pub undated: Vec<String>,
    pub ticks: Vec<(f64, f64)>,
}
pub fn layout(entries: impl IntoIterator<Item = (String, f64)>, width: f64) -> Timeline {
    let mut timeline = Timeline::default();
    let mut dated = Vec::new();
    for (hash, time) in entries {
        if time.is_finite() {
            dated.push((hash, time));
        } else {
            timeline.undated.push(hash);
        }
    }
    dated.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
    let Some((_, min)) = dated.first() else {
        return timeline;
    };
    let min = *min;
    let max = dated.last().unwrap().1;
    let span = max - min;
    for (hash, time) in dated {
        let position = if span == 0.0 {
            0.5
        } else {
            (time - min) / span
        };
        if let Some(last) = timeline.clusters.last_mut()
            && (position - last.position) * width.max(1.0) < 32.0
        {
            last.hashes.push(hash);
        } else {
            timeline.clusters.push(Cluster {
                position,
                hashes: vec![hash],
            });
        }
    }
    if span == 0.0 {
        timeline.ticks.push((0.5, min));
        return timeline;
    }
    let target = (width / 120.0).floor().clamp(2.0, 10.0);
    let intervals = [
        60_000.0,
        300_000.0,
        900_000.0,
        3_600_000.0,
        10_800_000.0,
        21_600_000.0,
        43_200_000.0,
        86_400_000.0,
        604_800_000.0,
        2_592_000_000.0,
        31_536_000_000.0,
    ];
    let step = intervals
        .into_iter()
        .find(|step| span / step <= target)
        .unwrap_or(span / target);
    let mut tick = (min / step).ceil() * step;
    while tick <= max {
        timeline.ticks.push(((tick - min) / span, tick));
        tick += step;
    }
    if timeline.ticks.is_empty() {
        timeline.ticks.push((0.0, min));
        timeline.ticks.push((1.0, max));
    }
    timeline
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proportional_time_positions_clusters_and_invalid_dates() {
        let axis = layout(
            [
                ("a".into(), 0.0),
                ("b".into(), 1000.0),
                ("c".into(), 10_000.0),
                ("d".into(), 10_000.0),
                ("undated".into(), f64::NAN),
            ],
            1000.0,
        );
        assert!((axis.clusters[1].position - 0.1).abs() < f64::EPSILON);
        assert_eq!(axis.clusters[2].hashes, ["c", "d"]);
        assert_eq!(axis.undated, ["undated"]);
        assert!(
            (layout([("one".into(), 4.0)], 100.0).clusters[0].position - 0.5).abs() < f64::EPSILON
        );
        assert_eq!(
            layout([("a".into(), 0.0), ("b".into(), 1.0)], 10.0)
                .clusters
                .len(),
            1
        );
    }
    #[test]
    fn same_day_ticks_are_independent_of_commit_count() {
        let axis = layout(
            (0..13).map(|i| (i.to_string(), f64::from(i) * 3_600_000.0)),
            800.0,
        );
        assert!(axis.ticks.len() < 13);
        assert_eq!(axis.clusters.len(), 13);
        assert!((axis.clusters[6].position - 0.5).abs() < f64::EPSILON);
    }
}

pub fn parse_timestamp(text: &str) -> Option<f64> {
    if !text.is_ascii() {
        return None;
    }
    let (date, time) = text.split_once('T')?;
    let parts = date.split('-').collect::<Vec<_>>();
    let [year, month, day] = parts.as_slice() else {
        return None;
    };
    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || ![year, month, day]
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let mut year = year.parse::<i32>().ok()?;
    let month = month.parse::<i32>().ok()?;
    let day = day.parse::<i32>().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return None,
    };
    if !(1..=days).contains(&day) {
        return None;
    }
    let (clock, offset) = if let Some(clock) = time.strip_suffix('Z') {
        (clock, 0)
    } else {
        if time.len() < 6 {
            return None;
        }
        let (clock, zone) = time.split_at(time.len() - 6);
        let sign = match zone.as_bytes()[0] {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        if zone.as_bytes()[3] != b':'
            || !zone[1..3]
                .bytes()
                .chain(zone[4..6].bytes())
                .all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let hours = zone[1..3].parse::<i32>().ok()?;
        let minutes = zone[4..6].parse::<i32>().ok()?;
        if hours > 23 || minutes > 59 {
            return None;
        }
        (clock, sign * (hours * 60 + minutes) * 60)
    };
    let parts = clock.split(':').collect::<Vec<_>>();
    let [hour, minute, second] = parts.as_slice() else {
        return None;
    };
    let (whole_second, fraction) = second
        .split_once('.')
        .map_or((*second, None), |(whole, fraction)| (whole, Some(fraction)));
    if hour.len() != 2
        || minute.len() != 2
        || whole_second.len() != 2
        || ![hour, minute, &whole_second]
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
        || fraction
            .is_some_and(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let hour = hour.parse::<i32>().ok()?;
    let minute = minute.parse::<i32>().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    let second = second.parse::<f64>().ok()?;
    if !second.is_finite() || !(0.0..60.0).contains(&second) {
        return None;
    }
    // Gregorian eras turn calendar dates into UTC days without browser normalization.
    year -= i32::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let days = era * 146097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year
        - 719468;
    Some(
        f64::from(days) * 86_400_000.0
            + f64::from(hour * 3600 + minute * 60 - offset) * 1000.0
            + second * 1000.0,
    )
}
#[cfg(test)]
mod timestamp_tests {
    use super::*;
    #[test]
    fn offsets_represent_the_same_instant_and_invalid_dates_stay_undated() {
        assert_eq!(
            parse_timestamp("2026-10-07T08:00:00-05:00"),
            parse_timestamp("2026-10-07T13:00:00Z")
        );
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0.0));
        for date in [
            "garbage",
            "2026-02-30T00:00:00Z",
            "2026-01-01T25:00:00Z",
            "2026-01-01T00:00:00+25:00",
            "2026-01-01T00:00:00+-1:00",
            "2026-01-01T00:00:00e1Z",
            "2026-+1-01T00:00:00Z",
        ] {
            assert!(parse_timestamp(date).is_none());
        }
        assert!(parse_timestamp("2024-02-29T00:00:00Z").is_some());
        assert!(parse_timestamp("2023-02-29T00:00:00Z").is_none());
    }
}
