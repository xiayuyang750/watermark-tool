"""抖音签名算法（纯 Python 移植）。

- a_bogus：移植自 douyin_parse (DLWangSan/douyin_parse) 的 abogus.py；
  原始实现源自 Evil0ctal/Douyin_TikTok_Download_API 与 JoeanAmier/TikTokDownloader（GPL v3）。
- X-Bogus：基于 Evil0ctal/Douyin_TikTok_Download_API 的 xbogus.py（Apache 2.0）。

依赖 gmssl 提供 SM3 哈希。
"""
import base64
import hashlib
import random
import re
import time
from urllib.parse import quote, urlencode

from gmssl import func, sm3

__all__ = ["ABogus", "XBogus"]


class ABogus:
    """生成抖音 Web API 请求所需的 a_bogus 签名参数。"""

    _url_encode_filter = re.compile(r"%([0-9A-F]{2})")
    _arguments = [0, 1, 14]
    _ua_key = "\x00\x01\x0e"
    _end_string = "cus"
    _version = [1, 0, 1, 5]
    _browser = "1536|742|1536|864|0|0|0|0|1536|864|1536|864|1536|742|24|24|MacIntel"
    _reg_init = [
        1937774191, 1226093241, 388252375, 3666478592,
        2842636476, 372324522, 3817729613, 2969243214,
    ]
    _charsets = {
        "s0": "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=",
        "s1": "Dkdpgh4ZKsQB80/Mfvw36XI1R25+WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe=",
        "s2": "Dkdpgh4ZKsQB80/Mfvw36XI1R25-WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe=",
        "s3": "ckdp1h4ZKsUB80/Mfvw36XIgR25+WQAlEi7NLboqYTOPuzmFjJnryx9HVGDaStCe",
        "s4": "Dkdpgh2ZmsQB80/MfvV36XI1R45-WUAlEixNLwoqYTOPuzKFjJnry79HbGcaStCe",
    }

    def __init__(self, platform: str = None):
        self.chunk = []
        self.size = 0
        self.reg = self._reg_init[:]
        # UA 特征码（需与请求实际使用的 User-Agent 对应，随平台算法更新）
        self.ua_code = [
            76, 98, 15, 131, 97, 245, 224, 133, 122, 199,
            241, 166, 79, 34, 90, 191, 128, 126, 122, 98,
            66, 11, 14, 40, 49, 110, 110, 173, 67, 96, 138, 252,
        ]
        self.browser = self._generate_browser_info(platform) if platform else self._browser
        self.browser_len = len(self.browser)
        self.browser_code = self._char_code_at(self.browser)

    # ---- 随机数生成 ----

    @classmethod
    def _list_1(cls, random_num=None, a=170, b=85, c=45):
        return cls._random_list(random_num, a, b, 1, 2, 5, c & a)

    @classmethod
    def _list_2(cls, random_num=None, a=170, b=85):
        return cls._random_list(random_num, a, b, 1, 0, 0, 0)

    @classmethod
    def _list_3(cls, random_num=None, a=170, b=85):
        return cls._random_list(random_num, a, b, 1, 0, 5, 0)

    @staticmethod
    def _random_list(a=None, b=170, c=85, d=0, e=0, f=0, g=0):
        r = a if a is not None else (random.random() * 10000)
        v = [r, int(r) & 255, int(r) >> 8]
        return [
            v[1] & b | d,
            v[1] & c | e,
            v[2] & b | f,
            v[2] & c | g,
        ]

    @staticmethod
    def _from_char_code(*args):
        return "".join(chr(code) for code in args)

    def _generate_string_1(self, r1=None, r2=None, r3=None):
        return (
            self._from_char_code(*self._list_1(r1))
            + self._from_char_code(*self._list_2(r2))
            + self._from_char_code(*self._list_3(r3))
        )

    # ---- 字符串 2 生成 ----

    def _generate_string_2(self, url_params: str, method="GET", start_time=0, end_time=0) -> str:
        a = self._list_4_list(url_params, method, start_time, end_time)
        e = self._end_check_num(a)
        a.extend(self.browser_code)
        a.append(e)
        return self._rc4_encrypt(self._from_char_code(*a), "y")

    def _list_4_list(self, url_params: str, method="GET", start_time=0, end_time=0) -> list:
        start_time = start_time or int(time.time() * 1000)
        end_time = end_time or (start_time + random.randint(4, 8))
        params_arr = self._generate_params_code(url_params)
        method_arr = self._generate_method_code(method)
        return self._list_4(
            (end_time >> 24) & 255, params_arr[21], self.ua_code[23],
            (end_time >> 16) & 255, params_arr[22], self.ua_code[24],
            (end_time >> 8) & 255, (end_time >> 0) & 255,
            (start_time >> 24) & 255, (start_time >> 16) & 255,
            (start_time >> 8) & 255, (start_time >> 0) & 255,
            method_arr[21], method_arr[22],
            (end_time // 256 // 256 // 256 // 256) & 0xFF,
            (start_time // 256 // 256 // 256 // 256) & 0xFF,
            self.browser_len,
        )

    @staticmethod
    def _list_4(a, b, c, d, e, f, g, h, i, j, k, m, n, o, p, q, r):
        return [
            44, a, 0, 0, 0, 0, 24, b, n, 0, c, d, 0, 0, 0, 1, 0, 239,
            e, o, f, g, 0, 0, 0, 0, h, 0, 0, 14, i, j, 0, k, m, 3, p, 1,
            q, 1, r, 0, 0, 0,
        ]

    # ---- SM3 哈希相关 ----

    @classmethod
    def _generate_method_code(cls, method: str) -> list:
        return cls._sm3_to_array(cls._sm3_to_array(method + cls._end_string))

    def _generate_params_code(self, params: str) -> list:
        return self._sm3_to_array(self._sm3_to_array(params + self._end_string))

    @classmethod
    def _sm3_to_array(cls, data) -> list:
        if isinstance(data, str):
            b = data.encode("utf-8")
        else:
            b = bytes(data)
        h = sm3.sm3_hash(func.bytes_to_list(b))
        return [int(h[i:i + 2], 16) for i in range(0, len(h), 2)]

    # ---- 工具方法 ----

    @staticmethod
    def _end_check_num(arr: list) -> int:
        r = 0
        for i in arr:
            r ^= i
        return r

    @staticmethod
    def _char_code_at(s: str) -> list:
        return [ord(c) for c in s]

    @staticmethod
    def _rc4_encrypt(plaintext: str, key: str) -> str:
        s = list(range(256))
        j = 0
        for i in range(256):
            j = (j + s[i] + ord(key[i % len(key)])) % 256
            s[i], s[j] = s[j], s[i]
        i = j = 0
        result = []
        for ch in plaintext:
            i = (i + 1) % 256
            j = (j + s[i]) % 256
            s[i], s[j] = s[j], s[i]
            t = (s[i] + s[j]) % 256
            result.append(chr(s[t] ^ ord(ch)))
        return "".join(result)

    @staticmethod
    def _generate_browser_info(platform: str) -> str:
        inner_w = random.randint(1280, 1920)
        inner_h = random.randint(720, 1080)
        outer_w = random.randint(inner_w, 1920)
        outer_h = random.randint(inner_h, 1080)
        values = [
            inner_w, inner_h, outer_w, outer_h,
            0, random.choice((0, 30)), 0, 0,
            outer_w, outer_h, outer_w, outer_h,
            inner_w, inner_h, 24, 24,
            platform,
        ]
        return "|".join(str(v) for v in values)

    # ---- 结果编码 ----

    @classmethod
    def _generate_result(cls, s: str, charset="s4") -> str:
        cs = cls._charsets[charset]
        result = []
        for i in range(0, len(s), 3):
            remaining = len(s) - i
            if remaining >= 3:
                n = (ord(s[i]) << 16) | (ord(s[i + 1]) << 8) | ord(s[i + 2])
                result.append(cs[(n >> 18) & 63])
                result.append(cs[(n >> 12) & 63])
                result.append(cs[(n >> 6) & 63])
                result.append(cs[n & 63])
            elif remaining == 2:
                n = (ord(s[i]) << 16) | (ord(s[i + 1]) << 8)
                result.append(cs[(n >> 18) & 63])
                result.append(cs[(n >> 12) & 63])
                result.append(cs[(n >> 6) & 63])
            else:
                n = ord(s[i]) << 16
                result.append(cs[(n >> 18) & 63])
                result.append(cs[(n >> 12) & 63])
        padding = (4 - len(result) % 4) % 4
        result.append("=" * padding)
        return "".join(result)

    # ---- 主接口 ----

    def get_value(self, url_params, method="GET", start_time=0, end_time=0,
                  random_num_1=None, random_num_2=None, random_num_3=None) -> str:
        """生成 a_bogus 签名（返回原始字符串，调用方需 quote 后拼入 URL）。"""
        if isinstance(url_params, dict):
            url_params = urlencode(url_params)
        string_1 = self._generate_string_1(random_num_1, random_num_2, random_num_3)
        string_2 = self._generate_string_2(url_params, method, start_time, end_time)
        return self._generate_result(string_1 + string_2, "s4")


class XBogus:
    """生成抖音 API 请求所需的 X-Bogus 签名参数。"""

    def __init__(self, user_agent: str = None) -> None:
        # 字符映射表: '0'-'9' (48-57) → 0-9, 'a'-'f' (97-102) → 10-15
        self._char_map = [None] * 128
        for i in range(10):
            self._char_map[48 + i] = i
        for i in range(6):
            self._char_map[97 + i] = 10 + i
        self._charset = "Dkdpgh4ZKsQB80/Mfvw36XI1R25-WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe="
        self._ua_key = b"\x00\x01\x0c"
        self.user_agent = user_agent or (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
            "(KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Edg/122.0.0.0"
        )

    def _md5_str_to_array(self, md5_str: str) -> list:
        if len(md5_str) > 32:
            return [ord(c) for c in md5_str]
        arr = []
        for i in range(0, len(md5_str), 2):
            high = self._char_map[ord(md5_str[i])]
            low = self._char_map[ord(md5_str[i + 1])]
            arr.append((high << 4) | low)
        return arr

    def _md5(self, data) -> str:
        if isinstance(data, str):
            arr = [ord(c) for c in data]
        elif isinstance(data, list):
            arr = data
        else:
            raise ValueError("Invalid input type")
        return hashlib.md5(bytes(arr)).hexdigest()

    def _md5_encrypt(self, url_path: str) -> list:
        first_md5 = self._md5(url_path)
        second_md5 = self._md5(self._md5_str_to_array(first_md5))
        return self._md5_str_to_array(second_md5)

    def _encoding_conversion(self, a, b, c, e, d, t, f, r, n, o, i, _, x, u, s, l, v, h, p):
        # 参数顺序是原算法的关键，必须严格匹配
        y = [a]
        y.append(int(i))
        y.extend([b, _, c, x, e, u, d, s, t, l, f, v, r, h, n, p, o])
        return bytes(y).decode("ISO-8859-1")

    @staticmethod
    def _encoding_conversion2(a: int, b: int, c: str) -> str:
        return chr(a) + chr(b) + c

    @staticmethod
    def _rc4_encrypt(key: bytes, data: bytes) -> bytearray:
        s = list(range(256))
        j = 0
        for i in range(256):
            j = (j + s[i] + key[i % len(key)]) % 256
            s[i], s[j] = s[j], s[i]
        i = j = 0
        encrypted = bytearray()
        for byte in data:
            i = (i + 1) % 256
            j = (j + s[i]) % 256
            s[i], s[j] = s[j], s[i]
            encrypted.append(byte ^ s[(s[i] + s[j]) % 256])
        return encrypted

    def _calc(self, a1: int, a2: int, a3: int) -> str:
        x = ((a1 & 255) << 16) | ((a2 & 255) << 8) | a3
        return (
            self._charset[(x >> 18) & 63]
            + self._charset[(x >> 12) & 63]
            + self._charset[(x >> 6) & 63]
            + self._charset[x & 63]
        )

    def get_xbogus(self, url_path: str) -> tuple:
        """生成 X-Bogus。返回 (带 X-Bogus 的参数字符串, X-Bogus 值, User-Agent)。"""
        rc4_encrypted = self._rc4_encrypt(
            self._ua_key, self.user_agent.encode("ISO-8859-1")
        )
        array1 = self._md5_str_to_array(
            self._md5(base64.b64encode(rc4_encrypted).decode("ISO-8859-1"))
        )
        array2 = self._md5_str_to_array(
            self._md5(self._md5_str_to_array("d41d8cd98f00b204e9800998ecf8427e"))
        )
        url_path_array = self._md5_encrypt(url_path)
        timer = int(time.time())
        ct = 536919696
        new_array = [
            64, 0.00390625, 1, 12,
            url_path_array[14], url_path_array[15],
            array2[14], array2[15],
            array1[14], array1[15],
            (timer >> 24) & 255, (timer >> 16) & 255,
            (timer >> 8) & 255, timer & 255,
            (ct >> 24) & 255, (ct >> 16) & 255,
            (ct >> 8) & 255, ct & 255,
        ]
        xor_result = new_array[0]
        for i in range(1, len(new_array)):
            b = new_array[i]
            if isinstance(b, float):
                b = int(b)
            xor_result ^= b
        new_array.append(xor_result)
        array3 = [new_array[i] for i in range(0, len(new_array), 2)]
        array4 = [new_array[i] for i in range(1, len(new_array), 2)]
        merge_array = array3 + array4
        garbled = self._encoding_conversion2(
            2, 255,
            self._rc4_encrypt(
                b"\xff",
                self._encoding_conversion(*merge_array).encode("ISO-8859-1"),
            ).decode("ISO-8859-1"),
        )
        xb = ""
        for i in range(0, len(garbled), 3):
            xb += self._calc(
                ord(garbled[i]), ord(garbled[i + 1]), ord(garbled[i + 2]),
            )
        return f"{url_path}&X-Bogus={xb}", xb, self.user_agent
