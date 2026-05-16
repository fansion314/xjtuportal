from __future__ import annotations

import argparse
import json
import logging
import os
from dataclasses import dataclass
from typing import Any

from Crypto.Cipher import AES
from Crypto.Util.Padding import pad, unpad


BASE_DIR = os.path.dirname(os.path.abspath(__file__))

logger = logging.getLogger(__name__)


class CryptoHandler:
    KEY = b"1234567890000000"
    IV = b"1234567890000000"

    @classmethod
    def encrypt(cls, data: dict[str, Any]) -> str:
        text = json.dumps(data, separators=(",", ":"), ensure_ascii=False)
        cipher = AES.new(cls.KEY, AES.MODE_CBC, cls.IV)
        return cipher.encrypt(pad(text.encode("utf-8"), AES.block_size)).hex()

    @classmethod
    def decrypt(cls, body: str) -> dict[str, Any]:
        hex_text = body.strip()
        if hex_text.startswith('"') and hex_text.endswith('"'):
            hex_text = json.loads(hex_text)

        cipher = AES.new(cls.KEY, AES.MODE_CBC, cls.IV)
        plaintext = unpad(cipher.decrypt(bytes.fromhex(hex_text)), AES.block_size)
        return json.loads(plaintext.decode("utf-8"))


@dataclass(frozen=True)
class PortalEndpoints:
    gateway: str = "http://10.184.6.32"

    @classmethod
    def from_config(cls, config: dict[str, Any]) -> "PortalEndpoints":
        network = config.get("network", {})
        gateway = network.get("gateway", cls.gateway)
        if not gateway.startswith(("http://", "https://")):
            gateway = f"http://{gateway}"
        return cls(gateway=gateway.rstrip("/"))

    @property
    def session_list_url(self) -> str:
        return f"{self.gateway}/portal-conversion/api/v3/session/list"

    @property
    def acct_unique_id_url(self) -> str:
        return f"{self.gateway}/portal-conversion/api/v3/session/acctUniqueId"


class PortalSessionClient:
    def __init__(
        self,
        token: str,
        endpoints: PortalEndpoints | None = None,
        session: Any | None = None,
        cookie: str | None = None,
        timeout: float = 5.0,
    ) -> None:
        import requests

        self.token = token
        self.endpoints = endpoints or PortalEndpoints()
        self.session = session or requests.Session()
        self.timeout = timeout
        self.session.headers.update(
            {
                "Accept": "application/json, text/plain, */*",
                "Accept-Language": "zh-CN,zh;q=0.9",
                "Authorization": token,
                "Cache-Control": "no-cache",
                "Content-Type": "application/json",
                "Origin": self.endpoints.gateway,
                "Pragma": "no-cache",
                "Referer": f"{self.endpoints.gateway}/wenet/auth",
                "User-Agent": (
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
                    "AppleWebKit/537.36 (KHTML, like Gecko) "
                    "Chrome/148.0.0.0 Safari/537.36 Edg/148.0.0.0"
                ),
            }
        )
        if cookie:
            self.session.headers["Cookie"] = cookie

    def list_sessions(self) -> dict[str, Any]:
        response = self.session.post(
            self.endpoints.session_list_url,
            data=b"",
            timeout=self.timeout,
        )
        response.raise_for_status()
        return CryptoHandler.decrypt(response.text)

    def logout_by_acct_unique_id(self, acct_unique_id: str, mac: str) -> None:
        payload = {
            "acctUniqueId": acct_unique_id,
            "mac": mac,
        }
        response = self.session.post(
            self.endpoints.acct_unique_id_url,
            data=CryptoHandler.encrypt(payload),
            timeout=self.timeout,
        )
        response.raise_for_status()
        if response.text.strip():
            logger.debug("acctUniqueId response body: %s", response.text)


def load_client(config_path: str, token: str, cookie: str | None = None) -> PortalSessionClient:
    import toml

    config = toml.load(config_path)
    endpoints = PortalEndpoints.from_config(config)
    timeout = float(config.get("network", {}).get("timeout_secs", 5))
    return PortalSessionClient(
        token=token,
        endpoints=endpoints,
        cookie=cookie,
        timeout=timeout,
    )


def print_sessions(data: dict[str, Any]) -> None:
    print(f"concurrency: {data.get('concurrency')}")
    for index, session in enumerate(data.get("sessions", []), start=1):
        print(
            f"{index}. {session.get('deviceType', '-'):<7} "
            f"{session.get('framed_ip_address', '-'):>15} "
            f"{session.get('calling_station_id', '-')} "
            f"{session.get('acct_start_time', '-')} "
            f"{session.get('acct_unique_id', '-')}"
        )


def self_test() -> None:
    request_payload = {
        "acctUniqueId": "radius:acct:test:xx:00000000000000000000000000000000",
        "mac": "00-11-22-33-44-55",
    }
    encrypted = CryptoHandler.encrypt(request_payload)
    assert CryptoHandler.decrypt(encrypted) == request_payload

    list_payload = {
        "concurrency": "1",
        "sessions": [
            {
                "deviceType": "PC",
                "framed_ip_address": "192.0.2.10",
                "calling_station_id": "00-11-22-33-44-55",
                "acct_start_time": "2026-01-01 00:00:00",
                "acct_unique_id": request_payload["acctUniqueId"],
            }
        ],
    }
    assert CryptoHandler.decrypt(CryptoHandler.encrypt(list_payload)) == list_payload
    print("self-test passed")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Reverse-engineered XJTU portal v3 session APIs."
    )
    parser.add_argument(
        "--config",
        default=os.path.join(BASE_DIR, "config.toml"),
        help="TOML config containing [network].gateway and timeout_secs.",
    )
    parser.add_argument("--token", help="Authorization token from the login response.")
    parser.add_argument(
        "--cookie",
        help="Optional raw Cookie header copied from browser/devtools.",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Verify crypto round-trip with synthetic non-sensitive payloads.",
    )

    subparsers = parser.add_subparsers(dest="command")
    subparsers.add_parser("list", help="POST /portal-conversion/api/v3/session/list")

    logout_parser = subparsers.add_parser(
        "logout", help="POST /portal-conversion/api/v3/session/acctUniqueId"
    )
    logout_parser.add_argument("acct_unique_id")
    logout_parser.add_argument("mac")

    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")

    if args.self_test:
        self_test()
        return

    if not args.token:
        parser.error("--token is required unless --self-test is used")
    if not args.command:
        parser.error("a command is required: list or logout")

    client = load_client(args.config, args.token, args.cookie)
    if args.command == "list":
        print_sessions(client.list_sessions())
    elif args.command == "logout":
        client.logout_by_acct_unique_id(args.acct_unique_id, args.mac)
        print("logout request accepted")


if __name__ == "__main__":
    main()
