#!/usr/bin/env python3
"""CLI for creating countries index and loading test data."""

import argparse
import valkey
from valkey.cluster import ValkeyCluster


def create_index(client, index_name, prefix) -> None:
    """Create FT index on country TAG field."""
    try:
        client.execute_command("FT.DROPINDEX", index_name)

        print("Dropped existing index")
    except Exception:
        pass
    
    result = client.execute_command(
        "FT.CREATE", index_name,
        "ON", "HASH",
        "PREFIX", "1", prefix,
        "SCHEMA", "country", "TAG"
    )

    print(f"FT.CREATE: {result}")


def load_data(client) -> None:
    """Load sample country data."""
    countries = ["USA", "GBR", "CAN", "FRA", "DEU"]
    for i, code in enumerate(countries, 1):
        result = client.execute_command("HSET", f"country:{i}", "country", code)
        print(f"HSET country:{i} -> {code}: {result}")




def query_data(client) -> None:
    """Run test queries to demonstrate TAG OR bug."""
    print("\n=== Query Tests ===")
    
    # Single tag query
    print("\nQuery 1: @country:{USA}")
    result = client.execute_command("FT.SEARCH", "countries_idx", "@country:{USA}")
    print(f"Result: {result}")
    
    # OR query (the bug)
    print("\nQuery 2: @country:{USA|GBR} (BUG: returns 0)")
    result = client.execute_command("FT.SEARCH", "countries_idx", "@country:{USA|GBR}")
    print(f"Result: {result}")
    
    # Workaround
    print("\nQuery 3: (@country:{USA} | @country:{GBR}) (workaround)")
    result = client.execute_command("FT.SEARCH", "countries_idx", "(@country:{USA} | @country:{GBR})")
    print(f"Result: {result}")



    
    # Add test data
    assert client.execute_command("HSET", "country2:1", "country2", "USA") == 1
    assert client.execute_command("HSET", "country2:2", "country2", "GBR") == 1
    assert client.execute_command("HSET", "country2:3", "country2", "CAN") == 1
    assert client.execute_command("HSET", "country2:4", "country2", "FRA") == 1
    assert client.execute_command("HSET", "country2:5", "country2", "DEU") == 1
    
    # Test tag OR syntax: @country2:{USA|GBR|CAN}
    result = client.execute_command("FT.SEARCH", "countries_idx2", "@country2:{USA|GBR|CAN}")
    assert result[0] == 3  # Should find 3 countries


def main():
    parser = argparse.ArgumentParser(description="Countries index CLI")
    parser.add_argument("-H", "--host", default="localhost")
    parser.add_argument("-p", "--port", type=int, default=6379)
    parser.add_argument("--cluster", action="store_true", help="Use cluster mode")
    parser.add_argument("command", choices=["create", "load", "query", "all"])
    
    args = parser.parse_args()
    
    if args.cluster:
        client = ValkeyCluster(host=args.host, port=args.port)
    else:
        client = valkey.Valkey(host=args.host, port=args.port)
    
    if args.command in ("create", "all"):
        create_index(client)
    if args.command in ("load", "all"):
        load_data(client)
    if args.command in ("query", "all"):
        query_data(client)
    # query_data_integration_test(client)


if __name__ == "__main__":
    main()