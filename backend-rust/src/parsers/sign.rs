//! 抖音签名算法（对齐 Python backend/app/parsers/douyin_sig.py）。
//!
//! - XBogus（Apache 2.0）：无外部依赖（MD5 + RC4 + 自定义编码），作纯 HTTP 备用通道签名。
//! - ABogus（GPL v3，依赖 SM3）：许可证传染，不移植；纯 HTTP 备用通道统一使用 XBogus。

/// 字符映射表: '0'-'9' (48-57) → 0-9, 'a'-'f' (97-102) → 10-15
const CHAR_MAP: [u8; 128] = {
    let mut m = [0u8; 128];
    let mut i = 0;
    while i < 10 {
        m[48 + i] = i as u8;
        i += 1;
    }
    while i < 16 {
        m[97 + (i - 10)] = i as u8;
        i += 1;
    }
    m
};

const CHARSET: &[u8] = b"Dkdpgh4ZKsQB80/Mfvw36XI1R25-WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe=";
const UA_KEY: [u8; 3] = [0x00, 0x01, 0x0c];
const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Edg/122.0.0.0";

pub struct XBogus {
    user_agent: String,
}

impl XBogus {
    pub fn new(user_agent: Option<String>) -> Self {
        XBogus {
            user_agent: user_agent.unwrap_or_else(|| DEFAULT_UA.to_string()),
        }
    }

    /// hex 字符串 → 字节数组（对应 Python _md5_str_to_array）
    fn md5_str_to_array(md5_str: &str) -> Vec<u8> {
        if md5_str.len() > 32 {
            return md5_str.as_bytes().to_vec();
        }
        let bytes = md5_str.as_bytes();
        let mut arr = Vec::with_capacity(bytes.len() / 2);
        for i in (0..bytes.len()).step_by(2) {
            let high = CHAR_MAP[bytes[i] as usize];
            let low = CHAR_MAP[bytes[i + 1] as usize];
            arr.push((high << 4) | low);
        }
        arr
    }

    /// MD5（对应 Python _md5）：data 为 str（按 ASCII 字节）或字节列表 → md5 hex
    fn md5(data: &[u8]) -> String {
        use md5::Digest;
        format!("{:x}", md5::Md5::digest(data))
    }

    /// 对应 Python _md5_encrypt：url_path 经两次 md5 → 字节数组
    fn md5_encrypt(url_path: &str) -> Vec<u8> {
        let first_md5 = Self::md5(url_path.as_bytes());
        let second_md5 = Self::md5(&Self::md5_str_to_array(&first_md5));
        Self::md5_str_to_array(&second_md5)
    }

    /// RC4（对应 Python _rc4_encrypt）
    fn rc4_encrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
        let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
        let mut j: usize = 0;
        for i in 0..256 {
            j = (j + s[i] as usize + key[i % key.len()] as usize) % 256;
            s.swap(i, j);
        }
        let mut i: usize = 0;
        j = 0;
        let mut encrypted = Vec::with_capacity(data.len());
        for &byte in data {
            i = (i + 1) % 256;
            j = (j + s[i] as usize) % 256;
            s.swap(i, j);
            let t = (s[i] as usize + s[j] as usize) % 256;
            encrypted.push(byte ^ s[t]);
        }
        encrypted
    }

    /// 3 字节 → 4 字符 charset 编码（对应 Python _calc）
    fn calc(a1: u8, a2: u8, a3: u8) -> [u8; 4] {
        let x = ((a1 as u32) << 16) | ((a2 as u32) << 8) | a3 as u32;
        [
            CHARSET[((x >> 18) & 63) as usize],
            CHARSET[((x >> 12) & 63) as usize],
            CHARSET[((x >> 6) & 63) as usize],
            CHARSET[(x & 63) as usize],
        ]
    }

    /// 生成 X-Bogus。返回 (带 X-Bogus 的参数字符串, X-Bogus 值)。
    /// （M3 纯 HTTP 备用通道接入后使用；当前未调用）
    #[allow(dead_code)]
    pub fn get_xbogus(&self, url_path: &str) -> (String, String) {
        let timer = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        self.get_xbogus_at(url_path, timer)
    }

    /// 内部实现：可注入时间戳（与 Python 版一致，测试时固定时间便于对比）。
    fn get_xbogus_at(&self, url_path: &str, timer: u32) -> (String, String) {
        use base64::Engine as _;

        // array1 = md5_str_to_array(md5(base64(rc4(ua_key, user_agent))))
        let rc4_encrypted = Self::rc4_encrypt(&UA_KEY, self.user_agent.as_bytes());
        let b64 = base64::engine::general_purpose::STANDARD.encode(&rc4_encrypted);
        let array1 = Self::md5_str_to_array(&Self::md5(b64.as_bytes()));

        // array2 = md5_str_to_array(md5(md5_str_to_array("d41d8...")))（空串 md5 常量）
        let array2 = Self::md5_str_to_array(&Self::md5(
            &Self::md5_str_to_array("d41d8cd98f00b204e9800998ecf8427e"),
        ));

        let url_path_array = Self::md5_encrypt(url_path);
        let ct: u32 = 536919696;

        let mut new_array: Vec<u32> = vec![
            64,
            0, // Python 中为 float 0.00390625，xor 时 int() 后为 0
            1,
            12,
            url_path_array[14] as u32,
            url_path_array[15] as u32,
            array2[14] as u32,
            array2[15] as u32,
            array1[14] as u32,
            array1[15] as u32,
            (timer >> 24) & 255,
            (timer >> 16) & 255,
            (timer >> 8) & 255,
            timer & 255,
            (ct >> 24) & 255,
            (ct >> 16) & 255,
            (ct >> 8) & 255,
            ct & 255,
        ];
        let mut xor_result = new_array[0];
        for &b in new_array.iter().skip(1) {
            xor_result ^= b;
        }
        new_array.push(xor_result);

        let array3: Vec<u32> = new_array.iter().step_by(2).copied().collect();
        let array4: Vec<u32> = new_array.iter().skip(1).step_by(2).copied().collect();
        let merge: Vec<u32> = array3.into_iter().chain(array4).collect();

        // Python: y = [a, int(i), b, _, c, x, e, u, d, s, t, l, f, v, r, h, n, p, o]
        // 其中参数按位置展开：a=merge[0], b=merge[1], c=merge[2], e=merge[3], d=merge[4],
        // t=merge[5], f=merge[6], r=merge[7], n=merge[8], o=merge[9], i=merge[10](float→0),
        // _=merge[11], x=merge[12], u=merge[13], s=merge[14], l=merge[15], v=merge[16],
        // h=merge[17], p=merge[18]
        let mut y = Vec::with_capacity(19);
        y.push(merge[0] as u8); // a
        y.push(0); // int(merge[10])（float 0.00390625）
        y.push(merge[1] as u8); // b
        y.push(merge[11] as u8); // _
        y.push(merge[2] as u8); // c
        y.push(merge[12] as u8); // x
        y.push(merge[3] as u8); // e
        y.push(merge[13] as u8); // u
        y.push(merge[4] as u8); // d
        y.push(merge[14] as u8); // s
        y.push(merge[5] as u8); // t
        y.push(merge[15] as u8); // l
        y.push(merge[6] as u8); // f
        y.push(merge[16] as u8); // v
        y.push(merge[7] as u8); // r
        y.push(merge[17] as u8); // h
        y.push(merge[8] as u8); // n
        y.push(merge[18] as u8); // p
        y.push(merge[9] as u8); // o

        let rc4ed = Self::rc4_encrypt(&[0xff], &y);
        let mut garbled = Vec::with_capacity(rc4ed.len() + 2);
        garbled.push(2); // encoding_conversion2 的 a
        garbled.push(255); // encoding_conversion2 的 b
        garbled.extend_from_slice(&rc4ed);

        let mut xb = String::new();
        for chunk in garbled.chunks(3) {
            if chunk.len() == 3 {
                let s = Self::calc(chunk[0], chunk[1], chunk[2]);
                xb.push_str(std::str::from_utf8(&s).unwrap());
            }
        }
        (format!("{url_path}&X-Bogus={xb}"), xb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定 UA + 固定时间戳，与 Python 版 douyin_sig.py 同参数输出对比（M2 验收）。
    #[test]
    fn xbogus_matches_python_sample() {
        let x = XBogus::new(Some(DEFAULT_UA.to_string()));
        let (_signed, xb) = x.get_xbogus_at(
            "https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=123456",
            1700000000,
        );
        assert_eq!(
            xb,
            "DFSzswVYkabANxTQtmWx-e9WX7rJ", // 2026-08-08 由 Python 版 douyin_sig.py 同参数（固定 UA+时间戳 1700000000）生成
            "X-Bogus 应与 Python 版逐字符一致"
        );
    }

    #[test]
    fn rc4_deterministic() {
        let out1 = XBogus::rc4_encrypt(&[0xff], b"hello");
        let out2 = XBogus::rc4_encrypt(&[0xff], b"hello");
        assert_eq!(out1, out2);
        assert_eq!(out1.len(), 5);
    }

    #[test]
    fn md5_str_to_array_roundtrip() {
        // md5("") = d41d8cd98f00b204e9800998ecf8427e → 16 字节
        let arr = XBogus::md5_str_to_array("d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(arr.len(), 16);
        assert_eq!(arr[0], 0xd4);
        assert_eq!(arr[15], 0x7e);
    }
}
