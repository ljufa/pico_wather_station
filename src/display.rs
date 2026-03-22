use core::fmt::Write;

use embassy_rp::rtc::DateTime;
use embedded_graphics::primitives::{Line, PrimitiveStyleBuilder, Rectangle, StrokeAlignment};

use crate::{model::Display, model::WeatherForecast, FmtBuf};
use u8g2_fonts::types::{FontColor, HorizontalAlignment, VerticalPosition};
#[cfg(feature = "debug")]
use {defmt_rtt as _, panic_probe as _};

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::prelude::{Point, RgbColor};
use u8g2_fonts::{fonts, FontRenderer};
use micromath::F32Ext;

const FG: Rgb565 = Rgb565::WHITE;
const FG_DIM: Rgb565 = Rgb565::CSS_LIGHT_GRAY;
const FG_MUTED: Rgb565 = Rgb565::CSS_DARK_GRAY;
const BG: Rgb565 = Rgb565::BLACK;

/// Draw only the time (HH:MM), clears just the time area (x=0..230)
pub fn draw_time(
    display: &mut Display,
    buf: &mut FmtBuf,
    date_time: Option<&DateTime>,
) {
    display.fill_solid(
        &Rectangle::new(Point { x: 0, y: 0 }, Size::new(230, 72)),
        BG,
    ).ok();

    let font_time = FontRenderer::new::<fonts::u8g2_font_logisoso58_tn>();

    if let Some(dt) = date_time {
        buf.reset();
        _ = write!(buf, "{:0>2}:{:0>2}", dt.hour, dt.minute);
        font_time
            .render(
                buf.as_str(),
                Point { x: 10, y: 5 },
                VerticalPosition::Top,
                FontColor::Transparent(FG),
                display,
            )
            .ok();
    }
}

/// Draw only the date + city, clears just the date area (x=230..480)
pub fn draw_date(
    display: &mut Display,
    buf: &mut FmtBuf,
    date_time: Option<&DateTime>,
    city: &str,
) {
    display.fill_solid(
        &Rectangle::new(Point { x: 230, y: 0 }, Size::new(250, 72)),
        BG,
    ).ok();

    let font_date = FontRenderer::new::<fonts::u8g2_font_logisoso38_tn>();

    if let Some(dt) = date_time {
        buf.reset();
        _ = write!(buf, "{:0>2}.{:0>2}.{}", dt.day, dt.month, dt.year);
        font_date
            .render(
                buf.as_str(),
                Point { x: 235, y: 5 },
                VerticalPosition::Top,
                FontColor::Transparent(FG),
                display,
            )
            .ok();
    }

    if !city.is_empty() {
        let font_city = FontRenderer::new::<fonts::u8g2_font_helvR18_tf>();
        font_city
            .render(
                city,
                Point { x: 235, y: 48 },
                VerticalPosition::Top,
                FontColor::Transparent(FG_DIM),
                display,
            )
            .ok();
    }
}

/// Draw everything below row 1 (lines, weather, forecast: y=73..320)
pub fn draw_weather(
    display: &mut Display,
    buf: &mut FmtBuf,
    forecast: &WeatherForecast,
) {
    let line_style = PrimitiveStyleBuilder::new()
        .stroke_color(FG)
        .stroke_width(2)
        .stroke_alignment(StrokeAlignment::Inside)
        .build();

    let font_text = FontRenderer::new::<fonts::u8g2_font_logisoso42_tf>();
    let font_big = FontRenderer::new::<fonts::u8g2_font_logisoso78_tn>();
    let font_label = FontRenderer::new::<fonts::u8g2_font_helvB18_tf>();
    let font_hour_label = FontRenderer::new::<fonts::u8g2_font_helvR18_tf>();
    let font_hour_temp = FontRenderer::new::<fonts::u8g2_font_logisoso28_tf>();
    let font_icon_sm = FontRenderer::new::<fonts::u8g2_font_open_iconic_weather_2x_t>();
    let font_icon_lg = FontRenderer::new::<fonts::u8g2_font_open_iconic_weather_4x_t>();

    // Clear weather area
    display.fill_solid(
        &Rectangle::new(Point { x: 0, y: 73 }, Size::new(480, 247)),
        BG,
    ).ok();

    // === Line ===
    Line::new(Point { x: 10, y: 73 }, Point { x: 470, y: 73 })
        .into_styled(line_style)
        .draw(display)
        .ok();

    // === Row 2: Current temp + hourly temps ===
    if !forecast.city.is_empty() {
        buf.reset();
        _ = write!(buf, "{:02}", forecast.temp.round() as i32);
        font_big
            .render(
                buf.as_str(),
                Point { x: 15, y: 85 },
                VerticalPosition::Top,
                FontColor::Transparent(FG),
                display,
            )
            .ok();

        buf.reset();
        _ = write!(buf, "\u{00b0}C");
        font_label
            .render(
                buf.as_str(),
                Point { x: 120, y: 80 },
                VerticalPosition::Top,
                FontColor::Transparent(FG_DIM),
                display,
            )
            .ok();
    }

    // Hourly temps
    if !forecast.hours.is_empty() {
        let hour_count = forecast.hours.len().min(5) as i32;
        let start_x = 165;
        let end_x = 475;
        let col_w = (end_x - start_x) / hour_count;
        for (i, hour) in forecast.hours.iter().take(hour_count as usize).enumerate() {
            let cx = start_x + (i as i32) * col_w + col_w / 2;

            buf.reset();
            _ = write!(buf, "{}", hour.h);
            font_hour_label
                .render_aligned(
                    buf.as_str(),
                    Point { x: cx - 8, y: 90 },
                    VerticalPosition::Top,
                    HorizontalAlignment::Center,
                    FontColor::Transparent(FG_DIM),
                    display,
                )
                .ok();

            if hour.i != '\0' {
                buf.reset();
                _ = write!(buf, "{}", hour.i);
                font_icon_sm
                    .render(
                        buf.as_str(),
                        Point { x: cx + 8, y: 90 },
                        VerticalPosition::Top,
                        FontColor::Transparent(FG_DIM),
                        display,
                    )
                    .ok();
            }

            buf.reset();
            _ = write!(buf, "{:02}\u{00b0}", hour.t);
            font_hour_temp
                .render_aligned(
                    buf.as_str(),
                    Point { x: cx, y: 120 },
                    VerticalPosition::Top,
                    HorizontalAlignment::Center,
                    FontColor::Transparent(FG),
                    display,
                )
                .ok();
        }
    }

    // === Line ===
    Line::new(Point { x: 10, y: 190 }, Point { x: 470, y: 190 })
        .into_styled(line_style)
        .draw(display)
        .ok();

    // === Row 3: Forecast ===
    let col_width = 120;
    for (i, day) in forecast.days.iter().enumerate() {
        let cx = (i as i32) * col_width + col_width / 2;

        font_label
            .render_aligned(
                day.n.as_str(),
                Point { x: cx - 13, y: 200 },
                VerticalPosition::Top,
                HorizontalAlignment::Center,
                FontColor::Transparent(FG_DIM),
                display,
            )
            .ok();

        buf.reset();
        _ = write!(buf, "{}", if day.rain > 0.0 { 'C' } else { 'E' });
        font_icon_lg
            .render(
                buf.as_str(),
                Point { x: cx + 22, y: 198 },
                VerticalPosition::Top,
                FontColor::Transparent(FG_DIM),
                display,
            )
            .ok();

        buf.reset();
        _ = write!(buf, "{:02}\u{00b0}", day.hi.round() as i32);
        font_text
            .render_aligned(
                buf.as_str(),
                Point { x: cx, y: 235 },
                VerticalPosition::Top,
                HorizontalAlignment::Center,
                FontColor::Transparent(FG),
                display,
            )
            .ok();

        buf.reset();
        _ = write!(buf, "{:02}\u{00b0}", day.lo.round() as i32);
        font_label
            .render_aligned(
                buf.as_str(),
                Point { x: cx, y: 290 },
                VerticalPosition::Top,
                HorizontalAlignment::Center,
                FontColor::Transparent(FG_MUTED),
                display,
            )
            .ok();
    }
}
