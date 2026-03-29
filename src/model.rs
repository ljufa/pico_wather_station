use embassy_rp::gpio::Output;
use embassy_time::Timer;
#[cfg(feature = "debug")]
use {defmt_rtt as _, panic_probe as _};
use embassy_rp::spi;
use heapless::{String, Vec};
use serde::Deserialize;

use embedded_graphics::pixelcolor::raw::RawU16;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;

/// ILI9488 display driver for Waveshare Pico-ResTouch-LCD-3.5.
///
/// This board uses 74HC4094 shift registers + 74HC4040 counter to convert
/// SPI serial data to 16-bit parallel. All SPI data must be sent in
/// 16-bit (2 byte) aligned chunks. CS must toggle between transactions
/// to reset the counter.
pub struct Ili9488Display {
    spi: spi::Spi<'static, embassy_rp::peripherals::SPI1, spi::Async>,
    dc: Output<'static>,
    cs: Output<'static>,
    rst: Output<'static>,
}

impl Ili9488Display {
    pub fn new(
        spi: spi::Spi<'static, embassy_rp::peripherals::SPI1, spi::Async>,
        dc: Output<'static>,
        cs: Output<'static>,
        rst: Output<'static>,
    ) -> Self {
        Self { spi, dc, cs, rst }
    }

    /// Send a command byte padded to 16 bits: [0x00, cmd]
    fn write_command(&mut self, cmd: u8) {
        self.dc.set_low();
        self.cs.set_low();
        self.spi.blocking_write(&[0x00, cmd]).ok();
        self.cs.set_high();
    }

    /// Send a command + parameters, each padded to 16 bits
    fn write_command_with_data(&mut self, cmd: u8, data: &[u8]) {
        self.dc.set_low();
        self.cs.set_low();
        self.spi.blocking_write(&[0x00, cmd]).ok();

        self.dc.set_high();
        // Pad each parameter byte to 16 bits
        let mut buf = [0u8; 64];
        for (i, &byte) in data.iter().enumerate() {
            buf[i * 2] = 0x00;
            buf[i * 2 + 1] = byte;
        }
        self.spi.blocking_write(&buf[..data.len() * 2]).ok();
        self.cs.set_high();
    }

    pub async fn init(&mut self) {
        // Hardware reset
        self.rst.set_high();
        Timer::after_millis(10).await;
        self.rst.set_low();
        Timer::after_millis(10).await;
        self.rst.set_high();
        Timer::after_millis(120).await;

        // Positive Gamma Control
        self.write_command_with_data(0xE0, &[
            0x00, 0x07, 0x0F, 0x0D, 0x1B, 0x0A, 0x3C, 0x78,
            0x4A, 0x07, 0x0E, 0x09, 0x1B, 0x1E, 0x0F,
        ]);
        // Negative Gamma Control
        self.write_command_with_data(0xE1, &[
            0x00, 0x22, 0x24, 0x06, 0x12, 0x07, 0x36, 0x47,
            0x47, 0x06, 0x0A, 0x07, 0x30, 0x37, 0x0F,
        ]);
        // Power Control 1
        self.write_command_with_data(0xC0, &[0x10, 0x10]);
        // Power Control 2
        self.write_command_with_data(0xC1, &[0x41]);
        // VCOM Control
        self.write_command_with_data(0xC5, &[0x00, 0x22, 0x80]);
        // Memory Access Control - Landscape 180° rotated (MY=1, MX=1, MV=1, BGR=1)
        self.write_command_with_data(0x36, &[0xE8]);
        // Interface Pixel Format - RGB565 (16-bit)
        self.write_command_with_data(0x3A, &[0x55]);
        // Interface Mode Control
        self.write_command_with_data(0xB0, &[0x00]);
        // Frame Rate Control
        self.write_command_with_data(0xB1, &[0xB0, 0x11]);
        // Display Inversion Control
        self.write_command_with_data(0xB4, &[0x02]);
        // Display Function Control
        self.write_command_with_data(0xB6, &[0x02, 0x02]);
        // Entry Mode Set
        self.write_command_with_data(0xB7, &[0xC6]);
        // Adjust Control 3
        self.write_command_with_data(0xF7, &[0xA9, 0x51, 0x2C, 0x82]);

        // Sleep Out
        self.write_command(0x11);
        Timer::after_millis(120).await;

        // Display On
        self.write_command(0x29);
        Timer::after_millis(20).await;
    }

    fn set_window(&mut self, x0: u16, y0: u16, x1: u16, y1: u16) {
        self.write_command_with_data(
            0x2A,
            &[
                (x0 >> 8) as u8,
                (x0 & 0xFF) as u8,
                (x1 >> 8) as u8,
                (x1 & 0xFF) as u8,
            ],
        );
        self.write_command_with_data(
            0x2B,
            &[
                (y0 >> 8) as u8,
                (y0 & 0xFF) as u8,
                (y1 >> 8) as u8,
                (y1 & 0xFF) as u8,
            ],
        );
    }

    /// Send Memory Write command and keep CS low for pixel streaming
    fn start_pixels(&mut self) {
        self.dc.set_low();
        self.cs.set_low();
        self.spi.blocking_write(&[0x00, 0x2C]).ok();
        self.dc.set_high();
    }

    fn end_pixels(&mut self) {
        self.cs.set_high();
    }

    pub fn clear_screen(&mut self, color: Rgb565) {
        self.set_window(0, 0, 479, 319);
        self.start_pixels();

        let raw = RawU16::from(color).into_inner();
        let bytes = raw.to_be_bytes();

        // Fill buffer with repeated pixel for efficient DMA transfer
        let mut buf = [0u8; 480]; // 240 pixels per chunk
        for i in 0..240 {
            buf[i * 2] = bytes[0];
            buf[i * 2 + 1] = bytes[1];
        }

        let total_pixels: u32 = 480 * 320;
        let pixels_per_chunk: u32 = 240;
        let chunks = total_pixels / pixels_per_chunk;
        let remaining = total_pixels % pixels_per_chunk;

        for _ in 0..chunks {
            self.spi.blocking_write(&buf).ok();
        }
        if remaining > 0 {
            self.spi
                .blocking_write(&buf[..(remaining as usize * 2)])
                .ok();
        }

        self.end_pixels();
    }


    pub fn set_inverted(&mut self, inverted: bool) {
        if inverted {
            self.write_command(0x21); // Display Inversion ON
        } else {
            self.write_command(0x20); // Display Inversion OFF
        }
    }
}

impl OriginDimensions for Ili9488Display {
    fn size(&self) -> Size {
        Size::new(480, 320)
    }
}

impl DrawTarget for Ili9488Display {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0 && point.x < 480 && point.y >= 0 && point.y < 320 {
                let x = point.x as u16;
                let y = point.y as u16;
                self.set_window(x, y, x, y);
                self.start_pixels();
                let raw = RawU16::from(color).into_inner();
                self.spi.blocking_write(&raw.to_be_bytes()).ok();
                self.end_pixels();
            }
        }
        Ok(())
    }

    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let area = area.intersection(&Rectangle::new(Point::zero(), self.size()));
        if area.size == Size::zero() {
            return Ok(());
        }

        let x0 = area.top_left.x as u16;
        let y0 = area.top_left.y as u16;
        let x1 = x0 + area.size.width as u16 - 1;
        let y1 = y0 + area.size.height as u16 - 1;

        self.set_window(x0, y0, x1, y1);
        self.start_pixels();

        for color in colors {
            let raw = RawU16::from(color).into_inner();
            self.spi.blocking_write(&raw.to_be_bytes()).ok();
        }

        self.end_pixels();
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let area = area.intersection(&Rectangle::new(Point::zero(), self.size()));
        if area.size == Size::zero() {
            return Ok(());
        }

        let x0 = area.top_left.x as u16;
        let y0 = area.top_left.y as u16;
        let x1 = x0 + area.size.width as u16 - 1;
        let y1 = y0 + area.size.height as u16 - 1;

        self.set_window(x0, y0, x1, y1);
        self.start_pixels();

        let raw = RawU16::from(color).into_inner();
        let bytes = raw.to_be_bytes();
        let count = area.size.width * area.size.height;

        let mut buf = [0u8; 480];
        let pixels_per_buf = buf.len() / 2;
        for i in 0..pixels_per_buf {
            buf[i * 2] = bytes[0];
            buf[i * 2 + 1] = bytes[1];
        }

        let full_bufs = count as usize / pixels_per_buf;
        let remaining = count as usize % pixels_per_buf;

        for _ in 0..full_bufs {
            self.spi.blocking_write(&buf).ok();
        }
        if remaining > 0 {
            self.spi.blocking_write(&buf[..remaining * 2]).ok();
        }

        self.end_pixels();
        Ok(())
    }
}

pub type Display = Ili9488Display;


#[derive(Deserialize, Clone)]
pub struct HourForecast {
    pub h: String<4>,
    pub t: i32,
    #[serde(default)]
    pub i: char,
}

#[derive(Deserialize, Clone)]
pub struct DayForecast {
    pub n: String<4>,
    pub lo: f32,
    pub hi: f32,
    #[serde(default)]
    pub rain: f32,
}

#[derive(Deserialize, Clone)]
pub struct WeatherForecast {
    pub city: String<30>,
    pub temp: f32,
    pub days: Vec<DayForecast, 4>,
    pub hours: Vec<HourForecast, 6>,
}

#[derive(Deserialize)]
pub struct InitialDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}
