#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

#[cfg(not(feature = "debug"))]
pub(crate) mod log_noop {
    macro_rules! info { ($($t:tt)*) => {} }
    macro_rules! error { ($($t:tt)*) => {} }
    macro_rules! unwrap { ($e:expr) => { $e.unwrap() } }
    pub(crate) use {info, error, unwrap};
}
#[cfg(not(feature = "debug"))]
use log_noop::{info, error, unwrap};

mod display;
mod fmtbuf;
mod model;
mod webapi;
use core::cell::RefCell;
use cyw43::JoinOptions;
use cyw43_pio::PioSpi;
#[cfg(feature = "debug")]
use defmt::{error, info, unwrap};
use embassy_executor::Spawner;
use embassy_net::{Config, Ipv4Address, Stack, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::pwm::{self, Pwm};
use model::Ili9488Display;

use embassy_rp::peripherals::{DMA_CH0, PIO0, RTC};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::rtc::Rtc;
use embassy_rp::rtc::{DateTime, DayOfWeek};

use embassy_sync::blocking_mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Timer};
use fmtbuf::FmtBuf;
use heapless::{String, Vec};
use model::{InitialDateTime, WeatherForecast};
use rand::RngCore;
use static_cell::StaticCell;
#[cfg(feature = "debug")]
use {defmt_rtt as _, panic_probe as _};
#[cfg(feature = "release")]
use panic_reset as _;

use embassy_rp::spi;

use u8g2_fonts::fonts;
use u8g2_fonts::types::{FontColor, VerticalPosition};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;

use embedded_alloc::LlffHeap as Heap;

// Test override for theme: None = auto (time-based), Some(true) = night, Some(false) = day
const FORCE_NIGHT: Option<bool> = None;
// const FORCE_NIGHT: Option<bool> = Some(true);   // force night
// const FORCE_NIGHT: Option<bool> = Some(false);  // force day

// Display brightness (0–100%) and night time interval, configurable via .env
const BRIGHTNESS_DAY: u16 = env_u16(env!("BRIGHTNESS_DAY"));
const BRIGHTNESS_NIGHT: u16 = env_u16(env!("BRIGHTNESS_NIGHT"));
const NIGHT_START: u8 = env_u8(env!("NIGHT_START"));
const NIGHT_END: u8 = env_u8(env!("NIGHT_END"));

const fn env_u16(s: &str) -> u16 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut acc: u16 = 0;
    while i < b.len() {
        acc = acc * 10 + (b[i] - b'0') as u16;
        i += 1;
    }
    acc
}
const fn env_u8(s: &str) -> u8 {
    env_u16(s) as u8
}

#[global_allocator]
static HEAP: Heap = Heap::empty();

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

static WEATHER_FORECAST: blocking_mutex::Mutex<
    CriticalSectionRawMutex,
    RefCell<WeatherForecast>,
> = blocking_mutex::Mutex::new(RefCell::new(WeatherForecast {
    city: String::new(),
    temp: 0.0,
    days: Vec::new(),
    hours: Vec::new(),
}));

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let rtc = Rtc::new(p.RTC);

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    // Display setup: GP10=CLK, GP11=MOSI, GP9=CS, GP8=DC, GP15=RST, GP13=BL
    let sck = p.PIN_10;
    let mosi = p.PIN_11;
    let disp_cs = Output::new(p.PIN_9, Level::High);
    let dc = Output::new(p.PIN_8, Level::High);
    let rst = Output::new(p.PIN_15, Level::High);

    let mut spi_config = spi::Config::default();
    spi_config.frequency = 40_000_000;
    let spi_disp = spi::Spi::new_txonly(p.SPI1, sck, mosi, p.DMA_CH1, spi_config);

    // PWM backlight on GP13
    let mut pwm_config = pwm::Config::default();
    pwm_config.top = 32768;
    pwm_config.compare_b = 32768;
    let backlight = Pwm::new_output_b(p.PWM_SLICE6, p.PIN_13, pwm_config.clone());

    let mut display = Ili9488Display::new(spi_disp, dc, disp_cs, rst);
    display.init().await;
    display.clear_screen(Rgb565::BLACK);

    let boot_font = u8g2_fonts::FontRenderer::new::<fonts::u8g2_font_logisoso42_tf>();
    boot_font
        .render(
            "Hello,",
            Point { x: 50, y: 100 },
            VerticalPosition::Top,
            FontColor::Transparent(Rgb565::WHITE),
            &mut display,
        )
        .ok();
    boot_font
        .render(
            "please wait...",
            Point { x: 50, y: 160 },
            VerticalPosition::Top,
            FontColor::Transparent(Rgb565::WHITE),
            &mut display,
        )
        .ok();

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.must_spawn(cyw43_task(runner));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let mut dns: Vec<Ipv4Address, 3> = Vec::new();
    let _ = dns.push(Ipv4Address::new(192, 168, 5, 20));
    let _ = dns.push(Ipv4Address::new(8, 8, 8, 8));
    let mut dhcp_config = embassy_net::DhcpConfig::default();
    dhcp_config.ignore_naks = true;
    let config = Config::dhcpv4(dhcp_config);
    let mut rng = RoscRng;
    let seed = rng.next_u64();

    static RESOURCES: StaticCell<StackResources<5>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        seed,
    );
    unwrap!(spawner.spawn(net_task(runner)));

    info!("Joining wifi network...");
    loop {
        match control
            .join(env!("WIFI_SSID"), JoinOptions::new(env!("WIFI_PASS").as_bytes()))
            .await
        {
            Ok(_) => {
                info!("Wifi joined!");
                break;
            }
            Err(err) => {
                info!("Join failed with status={}", err.status);
                Timer::after_secs(1).await;
            }
        }
    }
    while !stack.is_link_up() {
        info!("Waiting for link up...");
        Timer::after_millis(500).await;
    }
    info!("Link is up!");
    stack.wait_config_up().await;
    info!("Stack is up!");

    unwrap!(spawner.spawn(weather_forecast_task(stack)));
    unwrap!(spawner.spawn(display_task(display, rtc, stack, backlight)));
}

#[embassy_executor::task]
async fn display_task(
    mut display: Ili9488Display,
    mut rtc: Rtc<'static, RTC>,
    stack: Stack<'static>,
    mut backlight: Pwm<'static>,
) {
    // Fetch initial datetime
    let mut rx_buffer = [0; 2400];
    let body =
        webapi::make_api_request(stack, &mut rx_buffer, concat!(env!("GEOIP_API_URL"), "/datetime"))
            .await;
    if let Ok(now) = serde_json_core::de::from_str::<InitialDateTime>(body) {
        _ = rtc.set_datetime(DateTime {
            year: now.0.year,
            month: now.0.month,
            day: now.0.day,
            day_of_week: DayOfWeek::Monday,
            hour: now.0.hour,
            minute: now.0.minute,
            second: now.0.second,
        });
    }

    let mut buf = FmtBuf::new();
    let mut prev_night: Option<bool> = None;
    let mut prev_late_night: Option<bool> = None;
    let mut prev_temp: i32 = -999;
    let mut prev_minute: u8 = 255;
    let mut prev_day: u8 = 255;
    let mut weather_drawn = false;

    loop {
        let forecast = WEATHER_FORECAST.lock(|d| d.borrow().clone());
        let date_time = rtc.now().ok();

        // Night mode: 23:00–07:00
        let hour = date_time.as_ref().map(|dt| dt.hour).unwrap_or(12);
        let minute = date_time.as_ref().map(|dt| dt.minute).unwrap_or(0);
        let day = date_time.as_ref().map(|dt| dt.day).unwrap_or(0);
        let night = FORCE_NIGHT.unwrap_or(!(NIGHT_END..NIGHT_START).contains(&hour));

        // Handle night/day transition
        let late_night = night && hour < NIGHT_END;
        if Some(night) != prev_night || Some(late_night) != prev_late_night {
            let brightness = if night { BRIGHTNESS_NIGHT } else { BRIGHTNESS_DAY };
            let brightness = if late_night { brightness.saturating_sub(10) } else { brightness };
            let mut pwm_config = pwm::Config::default();
            pwm_config.top = 32768;
            pwm_config.compare_b = (32768u32 * brightness as u32 / 100) as u16;
            info!("Brightnes: {}", brightness);
            backlight.set_config(&pwm_config);
            display.set_inverted(night);
            prev_night = Some(night);
            prev_late_night = Some(late_night);
            weather_drawn = false;
        }

        // Redraw weather if data changed or first draw
        let cur_temp = forecast.temp as i32;
        if !weather_drawn || cur_temp != prev_temp {
            display.clear_screen(Rgb565::BLACK);
            display::draw_weather(&mut display, &mut buf, &forecast);
            display::draw_date(&mut display, &mut buf, date_time.as_ref(), &forecast.city);
            prev_temp = cur_temp;
            prev_day = day;
            prev_minute = 255; // force time redraw after full clear
            weather_drawn = true;
        }

        // Redraw date only if day changed
        if day != prev_day {
            display::draw_date(&mut display, &mut buf, date_time.as_ref(), &forecast.city);
            prev_day = day;
        }

        // Redraw time only if minute changed
        if minute != prev_minute {
            display::draw_time(&mut display, &mut buf, date_time.as_ref());
            prev_minute = minute;
        }

        // Check every 5 seconds
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn weather_forecast_task(stack: Stack<'static>) {
    loop {
        let mut rx_buffer = [0; 2400];
        let body = webapi::make_api_request(
            stack,
            &mut rx_buffer,
            concat!(env!("GEOIP_API_URL"), "/weather/forecast"),
        )
        .await;
        match serde_json_core::de::from_str::<WeatherForecast>(body) {
            Ok(forecast) => {
                WEATHER_FORECAST.lock(|d| d.replace(forecast.0));
            }
            Err(e) => {
                error!("Failed to parse forecast: {}", e);
            }
        }
        Timer::after(Duration::from_secs(600)).await;
    }
}

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}
