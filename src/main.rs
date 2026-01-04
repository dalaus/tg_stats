use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, Utc};
use clap::Parser;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

/// CLI аргументы
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Путь к файлу result.json
    #[arg(short, long)]
    file: PathBuf,

    /// Год для подведения итогов (например, 2023)
    #[arg(short, long)]
    year: i32,

    /// Часовой пояс в формате смещения (например, +0300, -0500, +0000)
    /// Это нужно, чтобы правильно определить год для сообщений на границе лет.
    #[arg(short, long, default_value = "+0000")]
    timezone: String,

    /// Количество выводимых сообщений
    #[arg(short, long, default_value_t = 5)]
    limit: usize,
}

// --- Структуры данных для парсинга JSON ---

#[derive(Debug, Deserialize)]
struct ChatExport {
    name: Option<String>,
    id: i64, // ID канала/чата
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    id: i64,
    #[serde(default)] // Некоторые системные сообщения могут не иметь даты
    date_unixtime: Option<String>,
    #[serde(default)]
    reactions: Vec<Reaction>,
    #[serde(default)]
    r#type: String, // service, message и т.д.
}

#[derive(Debug, Deserialize)]
struct Reaction {
    count: u32,
    // emoji и другие поля нам не важны
}

// Вспомогательная структура для обработанного сообщения
struct ProcessedMessage {
    id: i64,
    total_reactions: u32,
    date: DateTime<FixedOffset>,
}

// --- Логика ---

fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Парсим часовой пояс
    // Формат должен быть +HHMM или -HHMM (например, +0300 для Москвы)
    let tz_offset = if args.timezone.contains(':') {
        // Попытка распарсить, если пользователь ввел +03:00
        args.timezone.replace(":", "")
    } else {
        args.timezone.clone()
    }
    .parse::<i32>()
    .context("Неверный формат часового пояса. Используйте формат +0300")?;

    // Конвертируем числовое смещение (часы * 100 + минуты) в секунды для FixedOffset
    let offset_secs = (tz_offset / 100) * 3600 + (tz_offset % 100) * 60;
    let timezone =
        FixedOffset::east_opt(offset_secs).context("Некорректное смещение часового пояса")?;

    println!("Чтение файла {:?}...", args.file);
    let chat_data = load_chat_data(&args.file)?;

    println!("Найдено всего сообщений: {}", chat_data.messages.len());

    // 2. Фильтрация и обработка
    let mut relevant_messages = process_messages(&chat_data.messages, args.year, timezone);

    // 3. Сортировка (по убыванию реакций)
    relevant_messages.sort_by(|a, b| b.total_reactions.cmp(&a.total_reactions));

    // 4. Вывод топа
    let top_n = relevant_messages
        .into_iter()
        .take(args.limit)
        .collect::<Vec<_>>();

    println!(
        "\n--- Топ {} сообщений за {} год (TZ: {}) ---",
        args.limit, args.year, args.timezone
    );

    // ID канала в экспорте часто имеет префикс -100 или просто ID.
    // Для ссылок t.me/c/ используется "чистый" ID (без -100).
    let channel_id_str = chat_data.id.to_string();
    let clean_channel_id = if channel_id_str.starts_with("-100") {
        &channel_id_str[4..]
    } else {
        &channel_id_str
    };

    for (i, msg) in top_n.iter().enumerate() {
        let date_str = msg.date.format("%Y-%m-%d %H:%M").to_string();
        let link = format!("https://t.me/c/{}/{}", clean_channel_id, msg.id);

        println!(
            "{}. {} - {} (Реакций: {})",
            i + 1,
            date_str,
            link,
            msg.total_reactions
        );
    }

    Ok(())
}

/// Загрузка данных из JSON файла
fn load_chat_data(path: &PathBuf) -> Result<ChatExport> {
    let file = File::open(path).context("Не удалось открыть файл")?;
    let reader = BufReader::new(file);
    let chat: ChatExport = serde_json::from_reader(reader).context("Ошибка парсинга JSON")?;
    Ok(chat)
}

/// Фильтрация сообщений по году и подсчет реакций
fn process_messages(
    messages: &[Message],
    target_year: i32,
    tz: FixedOffset,
) -> Vec<ProcessedMessage> {
    messages
        .iter()
        .filter_map(|msg| {
            // Пропускаем сервисные сообщения
            if msg.r#type != "message" {
                return None;
            }

            // Парсим дату (unixtime string -> DateTime)
            let unix_timestamp = msg.date_unixtime.as_ref()?.parse::<i64>().ok()?;
            // Создаем DateTime<Utc> и конвертируем в целевой часовой пояс
            let dt_utc = DateTime::<Utc>::from_utc(
                NaiveDateTime::from_timestamp_opt(unix_timestamp, 0)?,
                Utc,
            );
            let dt_local = dt_utc.with_timezone(&tz);

            // Проверяем год
            if dt_local.year() != target_year {
                return None;
            }

            // Считаем сумму реакций
            let total_reactions: u32 = msg.reactions.iter().map(|r| r.count).sum();

            if total_reactions == 0 {
                return None; // Можно убрать, если интересны посты с 0 реакций
            }

            Some(ProcessedMessage {
                id: msg.id,
                total_reactions,
                date: dt_local,
            })
        })
        .collect()
}
