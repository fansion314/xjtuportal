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

LIST_RESPONSE_SAMPLE = (
    "143edf54f6b69bca7d42968f4a84d3fec66ca5cff3b36fe24eeebd4a61b36cccd25162a8f"
    "5afd0ce99be4ca8e21b9913af5614961b884e586735e591d8dab6902c4a7af6929730fb15b"
    "2ce3013968de600e18e4d6c0e0be744e3155f685ef7f5b990114a7f7fc733d92e4a248389"
    "cf9d316624ec03ef833b3f0d5cada4924d8f718a52aca09f566d46a659c7be803e5822ba5"
    "ef115ab35c353ae5981993d91df2c9920eaea1bc43fd5c3268b5da1843b905c968bc03eae"
    "871c5c9f0954d7b1310dffb065586ba5bb69e6d81f3a3dfafd35c725220df2a213e4bbe3"
    "a8be027352fdae6ba64d94d30657a1b4b86e4113136773494f6f211dd02a1dd8e66ac47f"
    "56f68cceaab066f29c5341ba008c7d5822963b0b83b2f06141056e8441147a6b30148f1"
    "d85128b5e225e36b40d771dde95676bf20eda4ce3c711647d070796b531eec57cd54c408"
    "f76930226fecbd378930e73c1414a9e742b7a042909ef22c40668504acfab94f6e62ee5e7"
    "09440a56714b8e922434b2a507756247bf0643f38e6b08492dcf0f14704765e2526bf6b8"
    "e7959503c64efc917d9fd685c2ad25b0db175d36d214e474a8fb2de7a57734a6cd8ce7a"
    "3ff11de906c9e0d6c2867743be0c5f691b6e9e3182aae42ab3c036c7f1e84986e3a56910"
    "1eed6be2789684d4d502ab1fa3078e1e3407f94918429884078c5fac54c577243af493e5"
    "d559352ddfb9cab449eae93b73365f5e72c475bddff56f6bbe88977f70686b5f7d14b366"
    "55fea53c230ebc9b0dc109dadfa6fbe83a3a30f599a3497f1b2a23f73999598cb007b538"
    "4ee7803d2947802ae217fd8ac59345d7d3f75156c8e5ded2a778f1a0fc443981365c51be"
    "004dbd716a66bc9d0f553aa7262aae42d0919d8affbf490a52b6d417cfc299fa29fb605a"
    "bdb974e283ccb264565490fcaef2e9dbf36271edc551fa29d038d265e6ab7e32930dbf7dd"
    "f0fb9b784ce5ce9c836f0d638dfd3c76756893a9ae797b27aa6122ade67890445ad73d05"
    "9d4cd466dd919d2e9b6b1bf004a9cccfc73d4944f2f43bcc070534f223c0fcde834f5bd5"
    "719d6644c2c8ad1d406b03a911d4f24cb6fa040c91ea235c7fd52af7dc17a71a46275a"
    "2c3552b9d98bc2732fcd91a0bc142a141f6946f519de65530f94814783713283986d23ef5"
    "786b47fe2b616074de84ce891f44af2f890426b70013a49be00009a77e34fb010d531777"
    "15216fbdb8a0453f85a6998f6a6f597df581abb36d2557e77a40c941eebede7d7c3f1d12"
    "8b7c35f1312454850e19c2b567c7f79e69d213db68555632140aef2b8bba978582d469d77"
    "f9a971782b26b127872ec9b74171811388587e9b66d6e55536001b07dcd28151338cf5c2"
    "1f33efa7f835ecd3d5112ea561df8392f9880204294a917aa96695eba72c594e55c3f449b"
    "c4f00fe5005144e6a7fdd1360d062f297e668237a9ee6bf210e4f2db2c338b073e4e8443"
    "5763adb558d29deb92b0022240203cc643fc69791d0bc17e4be314aff8899b50b69fb8ab"
    "d27ecc4c754fd6e2c7fd979c468d75f25f78bb8870aeaf7788557bfebaf7d2644443f99e"
    "3887fbe64a73f42fda0dd3947a646db4a1325617a8d72d16ca80d2c36517c178ded4364"
    "ec1af6de06e516d85fd8b0b99b1a3df817805c69f48dacca2ec36e2e0002c6fe330fe3e"
    "b506e8a6ec5a32ff4971b92eeaa472fe1ddffb7c62b3a3ffbfa"
)

ACCT_UNIQUE_ID_REQUEST_SAMPLE = (
    "cdff64ee931c95d99f80308b83a864916312771727ea1cf15fd7ca5815963b9931906c9ff"
    "7f552fd9ec31df128bc69d68d3567558ae5648c009aee1c555fdd3e03a43fbe31e61ef2"
    "b3fb6107c91f2dfeb254daa14ffd05840de71974a064712e517d42d6f48c0528331585e1"
    "cc79986e4a365d75a18c2061e0e5131dd226d72cdc6da7a9f8a13d0dc65625bd02cbbb9b"
)

SAMPLE_ACCT_UNIQUE_ID = (
    "radius:acct:86218bd4-ab5e-48e8-a760-6a853e94a3fc:xx:"
    "85186d7532b663beb9321de89e8e45fb"
)
SAMPLE_MAC = "9a-9e-a5-06-5c-03"

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
    list_payload = CryptoHandler.decrypt(LIST_RESPONSE_SAMPLE)
    assert list_payload["concurrency"] == "3"
    assert len(list_payload["sessions"]) == 3
    assert list_payload["sessions"][1]["acct_unique_id"] == SAMPLE_ACCT_UNIQUE_ID

    request_payload = {
        "acctUniqueId": SAMPLE_ACCT_UNIQUE_ID,
        "mac": SAMPLE_MAC,
    }
    assert CryptoHandler.encrypt(request_payload) == ACCT_UNIQUE_ID_REQUEST_SAMPLE
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
        help="Verify the crypto and payload layout against exp.md samples.",
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
