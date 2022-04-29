from dataclasses import dataclass
from typing import Dict, Any

Query = Any
Update = Any
nat64 = Any

__all__ = [
    "initialize_supply",
    "transfer",
    "balance",
    "ticker",
    "name",
    "total_supply",
]


@dataclass
class Account:
    address: str
    balance: nat64


@dataclass
class State:
    accounts: Dict[str, Account]
    total_supply: nat64
    ticker: str
    name: str


state = State(
    accounts={},
    total_supply=0,
    ticker="",
    name="KYBRA",
)


def initialize_supply(
    ticker: str, name: str, total_supply: nat64, original_address: str
) -> Update:
    global state
    state = State(
        accounts={
            original_address: Account(
                address=original_address,
                balance=total_supply,
            )
        },
        ticker=ticker,
        name=name,
        total_supply=total_supply,
    )

    return True


def transfer(from_: str, to: str, amount: nat64) -> Update:
    global state
    if state.accounts[to] is None:
        state.accounts[to] = Account(address=to, balance=0)

    state.accounts[from_].balance -= amount
    state.accounts[to].balance += amount

    return True


def balance(address: str) -> Query:
    account = state.accounts[address]
    if account is None:
        return 0
    return account.balance


def ticker() -> Query:
    return state.ticker


def name() -> Query:
    return state.name


def total_supply() -> Query:
    return state.total_supply


if __name__ == "__main__":
    print(name())
