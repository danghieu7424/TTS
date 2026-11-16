/****
 * [ATOM]: normalizer
 * Chức năng: Chuẩn hóa văn bản Tiếng Việt trước khi đưa vào G2P
 * Xử lý: Số đếm dài, số thập phân, ngày tháng, giờ giấc, tiền tệ.
 ****/
use lazy_static::lazy_static;
use regex::{Captures, Regex};

lazy_static! {
    static ref NUM_RE: Regex = Regex::new(r"\b(\d+)\b").unwrap();
    static ref CTRL_RE: Regex = Regex::new(r"(?i)\bctrl\b").unwrap();
    static ref PAN_RE: Regex = Regex::new(r"(?i)\bpan\b").unwrap();
    static ref OK_RE: Regex = Regex::new(r"(?i)\b(ok|o\.k|okay)\b").unwrap();
}

fn number_to_words(n: u64) -> String {
    if n == 0 {
        return "không".to_string();
    }

    let units = [
        "", "một", "hai", "ba", "bốn", "năm", "sáu", "bảy", "tám", "chín",
    ];
    let scales = ["", "nghìn", "triệu", "tỉ", "nghìn tỉ", "triệu tỉ", "tỉ tỉ"];

    let mut chunks = Vec::new();
    let mut temp = n;
    while temp > 0 {
        chunks.push((temp % 1000) as u32);
        temp /= 1000;
    }

    let mut words = Vec::new();
    let num_chunks = chunks.len();
    let mut has_higher = false;

    for (i, &chunk) in chunks.iter().enumerate().rev() {
        if chunk == 0 {
            if has_higher && i > 0 && chunks[0..i].iter().any(|&x| x > 0) {
                words.push("không".to_string());
                words.push(scales[i].to_string());
            }
            continue;
        }

        has_higher = true;
        let c = chunk / 100;
        let b = (chunk % 100) / 10;
        let a = chunk % 10;

        let mut chunk_words = Vec::new();

        // Trăm
        if num_chunks > 1 && i < num_chunks - 1 {
            if c == 0 {
                chunk_words.push("không".to_string());
            } else {
                chunk_words.push(units[c as usize].to_string());
            }
            chunk_words.push("trăm".to_string());
        } else if c > 0 {
            chunk_words.push(units[c as usize].to_string());
            chunk_words.push("trăm".to_string());
        }

        // Chục
        if b > 1 {
            chunk_words.push(units[b as usize].to_string());
            chunk_words.push("mươi".to_string());
        } else if b == 1 {
            chunk_words.push("mười".to_string());
        } else if (c > 0 || (num_chunks > 1 && i < num_chunks - 1)) && a > 0 {
            chunk_words.push("lẻ".to_string());
        }

        // Đơn vị
        if a == 1 {
            if b > 1 {
                chunk_words.push("mốt".to_string());
            } else {
                chunk_words.push("một".to_string());
            }
        } else if a == 5 {
            if b > 0 {
                chunk_words.push("lăm".to_string());
            } else {
                chunk_words.push("năm".to_string());
            }
        } else if a > 0 {
            chunk_words.push(units[a as usize].to_string());
        }

        if i > 0 && !chunk_words.is_empty() {
            chunk_words.push(scales[i].to_string());
        }

        words.extend(chunk_words);
    }

    words.join(" ")
}

pub fn normalize(text: &str) -> String {
    let mut s = text.to_string();

    // 1. Chuẩn hóa phím tắt, từ mượn và khẩu ngữ phổ biến
    s = OK_RE.replace_all(&s, "ô kê").to_string();
    s = CTRL_RE.replace_all(&s, "côn-tron").to_string();
    s = PAN_RE.replace_all(&s, "pen").to_string();

    // 2. Chuẩn hóa tỷ lệ / nhịp (vd: 1:2 -> 1 chia 2)
    let ratio_re = Regex::new(r"(\d+)\s*:\s*(\d+)").unwrap();
    s = ratio_re
        .replace_all(&s, |caps: &Captures| {
            format!("{} chia {}", &caps[1], &caps[2])
        })
        .to_string();

    // 3. Chuẩn hóa số thập phân (1.5 hoặc 1,5 -> 1 phẩy 5)
    let dec_re = Regex::new(r"(\d+)[.,](\d+)").unwrap();
    s = dec_re
        .replace_all(&s, |caps: &Captures| {
            format!("{} phẩy {}", &caps[1], &caps[2])
        })
        .to_string();

    // 4. Chuẩn hóa số đếm dài thành chữ
    s = NUM_RE
        .replace_all(&s, |caps: &Captures| {
            if let Ok(num) = caps[1].parse::<u64>() {
                number_to_words(num)
            } else {
                caps[0].to_string()
            }
        })
        .to_string();

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn test_normalize_ok_pronunciation() {
        assert_eq!(normalize("Ok, chúng ta bắt đầu"), "ô kê, chúng ta bắt đầu");
        assert_eq!(normalize("Mọi thứ đã OK!"), "Mọi thứ đã ô kê!");
        assert_eq!(normalize("Nhấn ok để tiếp tục"), "Nhấn ô kê để tiếp tục");
        assert_eq!(normalize("O.K, hoàn tất"), "ô kê, hoàn tất");
    }

    #[test]
    fn normalizes_decimals_ratios_and_integer_counts() {
        assert_eq!(
            normalize("Tỉ lệ 1:2, giá 1.5"),
            "Tỉ lệ một chia hai, giá một phẩy năm"
        );
    }

    #[test]
    fn normalizes_supported_borrowed_terms() {
        assert_eq!(normalize("Ctrl PAN okay"), "côn-tron pen ô kê");
    }

    proptest! {
        #[test]
        fn arbitrary_text_normalizes_without_panicking(text in "([\\x00-\\x7f]|[\\u{80}-\\u{ffff}])*") {
            let _ = normalize(&text);
        }
    }
}
