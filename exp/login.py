import requests
import json
import toml
import time
import socket
import logging
import os
from urllib.parse import urlparse, urlunparse
from Crypto.Cipher import AES
from Crypto.Util.Padding import pad, unpad

# ================= 配置日志 =================
# 获取脚本所在目录，确保日志文件生成在脚本同级目录下
BASE_DIR = os.path.dirname(os.path.abspath(__file__))
LOG_FILE = os.path.join(BASE_DIR, 'login_history.log')

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(message)s',
    handlers=[
        logging.FileHandler(LOG_FILE, encoding='utf-8'),  # 输出到文件
        logging.StreamHandler()  # 输出到控制台
    ]
)
logger = logging.getLogger(__name__)


# ================= 加密/解密工具类 =================
class CryptoHandler:
    KEY = b'1234567890000000'
    IV = b'1234567890000000'

    @classmethod
    def encrypt(cls, data_dict):
        try:
            # 关键点1：去除JSON中的空格
            text = json.dumps(data_dict, separators=(',', ':'))
            cipher = AES.new(cls.KEY, AES.MODE_CBC, cls.IV)
            encrypted_bytes = cipher.encrypt(pad(text.encode('utf-8'), AES.block_size))
            return encrypted_bytes.hex()
        except Exception as e:
            logger.error(f"加密失败: {e}")
            raise

    @classmethod
    def decrypt(cls, hex_str):
        try:
            # 关键点2：每次解密都需要新的 cipher 对象
            cipher = AES.new(cls.KEY, AES.MODE_CBC, cls.IV)
            # 处理可能的双引号包裹
            if hex_str.startswith('"') and hex_str.endswith('"'):
                hex_str = json.loads(hex_str)

            encrypted_bytes = bytes.fromhex(hex_str)
            decrypted_bytes = unpad(cipher.decrypt(encrypted_bytes), AES.block_size)
            return json.loads(decrypted_bytes.decode('utf-8'))
        except Exception as e:
            logger.error(f"解密失败: {e}")
            return None


# ================= 核心逻辑类 =================
class CampusLogin:
    def __init__(self, config_path):
        self.config = toml.load(config_path)
        self.session = requests.Session()
        # 伪装成浏览器
        self.session.headers.update({
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"
        })

    def get_local_ip(self):
        """获取本机内网IP (通过连接外部地址来判断出口IP)"""
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.connect(("8.8.8.8", 80))
            ip = s.getsockname()[0]
            s.close()
            return ip
        except Exception:
            return "127.0.0.1"

    def fix_redirect_url(self, raw_url):
        """修复网关返回的URL缺失斜杠的问题"""
        parsed = urlparse(raw_url)
        if not parsed.path:
            parsed = parsed._replace(path='/')
        return urlunparse(parsed)

    def check_network(self):
        """
        检查网络状态
        Return: (status_code, redirect_url or None)
        """
        try:
            url = self.config['network']['test_url']
            # 禁止自动跳转，以便捕获302
            resp = self.session.get(url, allow_redirects=False, timeout=5)

            # 情况1: 直接返回200 (极少见，现在大厂基本都强制HTTPS)
            if resp.status_code == 200:
                return "online", None

            # 情况2: 发生重定向 (这是最常见的情况)
            elif resp.status_code in [301, 302, 307] and 'Location' in resp.headers:
                location = resp.headers['Location']

                # 核心判断逻辑：看重定向去了哪里
                # A. 如果重定向到了百度自己的 HTTPS 地址 -> 说明网通了
                if "https://" in location:
                    logger.info(f"检测到 HTTP->HTTPS 正常跳转，网络已连通")
                    return "online", None

                # B. 如果重定向到了 10.x.x.x 或 portal 页面 -> 说明没网
                return "redirected", location

        except Exception as e:
            logger.error(f"testtest 网络检测异常: {e}")
            return "error", None

    def login(self, redirect_url):
        fixed_url = self.fix_redirect_url(redirect_url)
        logger.info(f"获取并修复重定向URL: {fixed_url}")

        # 构造 Payload (注意顺序)
        payload = {
            "deviceType": "PC",
            "redirectUrl": fixed_url,
            "webAuthUser": self.config['user']['username'],
            "webAuthPassword": self.config['user']['password']
        }

        # 加密
        encrypted_data = CryptoHandler.encrypt(payload)

        # 发送请求
        try:
            login_url = self.config['network']['login_url']
            headers = {"Content-Type": "application/json"}
            logger.info("正在发送登录请求...")

            resp = self.session.post(login_url, data=encrypted_data, headers=headers)

            if resp.status_code == 200:
                # 解密响应
                result = CryptoHandler.decrypt(resp.text)
                if result and result.get('code') == 0:  # 根据你之前的解密结果，成功是 error: 0 或 code: 0
                    token = result.get('token', '')
                    logger.info(f"✅ 登录成功! Token前缀: {token[:10]}...")
                    return True
                elif result and result.get('error') == 0:  # 兼容不同字段名
                    token = result.get('token', '')
                    logger.info(f"✅ 登录成功! Token前缀: {token[:10]}...")
                    return True
                else:
                    logger.error(f"❌ 业务逻辑失败: {json.dumps(result, ensure_ascii=False)}")
                    return False
            else:
                logger.error(f"❌ HTTP请求失败: {resp.status_code}")
                return False

        except Exception as e:
            logger.error(f"登录过程发生异常: {e}")
            return False

    def report_status(self):
        """上报本机信息到指定 Webhook"""
        try:
            webhook_url = self.config['webhook']['report_url']
            hostname = socket.gethostname()
            ip = self.get_local_ip()

            data = {
                "hostname": hostname,
                "ip": ip,
                "secret": self.config['webhook'].get('secret', ''),
                "message": "Campus Network Connected",
                "timestamp": time.strftime("%Y-%m-%d %H:%M:%S")
            }

            # 这里假设你的接收端接受 JSON POST
            requests.post(webhook_url, json=data, timeout=5)
            logger.info(f"📡 已上报本机信息: {hostname} - {ip}")
        except Exception as e:
            logger.warning(f"⚠️ 上报信息失败 (不影响上网): {e}")

    def run(self):
        logger.info(">>> 开始执行自动登录检测 <<<")
        status, redirect_url = self.check_network()

        if status == "online":
            logger.info("🌐 网络已连通，无需登录。")
            # 即使已连通，也可以选择上报一下IP，确保IP变动能被感知
            # self.report_status()
        elif status == "redirected":
            logger.info("检测到重定向，开始尝试登录...")
            if self.login(redirect_url) and self.config['webhook'].get('enabled', True):
                self.report_status()
        elif status == "error":
            logger.error("无法连接网络，请检查网线或WiFi连接。")
        else:
            logger.warning(f"未知的网络状态: Code={status}")


if __name__ == "__main__":
    # 配置文件路径
    config_file = os.path.join(BASE_DIR, 'config.toml')

    if not os.path.exists(config_file):
        logger.critical(f"配置文件未找到: {config_file}")
    else:
        bot = CampusLogin(config_file)
        bot.run()

