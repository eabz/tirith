"""Hidden acceptance: parse-money."""

import unittest
from decimal import Decimal

from tally.money import parse_money


class ParseMoneyTest(unittest.TestCase):
    def test_plain(self):
        self.assertEqual(parse_money(" 7 "), Decimal("7"))
        self.assertEqual(parse_money("12.50"), Decimal("12.50"))

    def test_symbols_and_separators(self):
        self.assertEqual(parse_money("$1,234.50"), Decimal("1234.50"))
        self.assertEqual(parse_money("€12.00"), Decimal("12.00"))
        self.assertEqual(parse_money("¥1,000"), Decimal("1000"))

    def test_trailing_currency_code(self):
        self.assertEqual(parse_money("1,234 JPY"), Decimal("1234"))
        self.assertEqual(parse_money("9.99 USD"), Decimal("9.99"))

    def test_negative(self):
        self.assertEqual(parse_money("(12.00)"), Decimal("-12.00"))
        self.assertEqual(parse_money("-3.5"), Decimal("-3.5"))

    def test_invalid(self):
        for text in ("", "abc", "12.3.4", "$", "1,2a", "USD"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                parse_money(text)
