-- typing lands on this comment line 
SELECT * FROM plan024.fixture_wide;
SELECT * FROM plan024.fixture_large;
SELECT * FROM plan024.fixture_many;

-- report 0000: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0000
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '1 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 100;

-- report 0001: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0001
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '2 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 101;

-- report 0002: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0002
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '3 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 102;

-- report 0003: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0003
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '4 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 103;

-- report 0004: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0004
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '5 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 104;

-- report 0005: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0005
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '6 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 105;

-- report 0006: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0006
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '7 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 106;

-- report 0007: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0007
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '8 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 107;

-- report 0008: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0008
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '9 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 108;

-- report 0009: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0009
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '10 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 109;

-- report 0010: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0010
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '11 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 110;

-- report 0011: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0011
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '12 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 111;

-- report 0012: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0012
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '13 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 112;

-- report 0013: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0013
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '14 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 113;

-- report 0014: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0014
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '15 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 114;

-- report 0015: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0015
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '16 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 115;

-- report 0016: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0016
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '17 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 116;

-- report 0017: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0017
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '18 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 117;

-- report 0018: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0018
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '19 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 118;

-- report 0019: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0019
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '20 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 119;

-- report 0020: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0020
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '21 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 120;

-- report 0021: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0021
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '22 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 121;

-- report 0022: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0022
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '23 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 122;

-- report 0023: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0023
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '24 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 123;

-- report 0024: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0024
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '25 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 124;

-- report 0025: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0025
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '26 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 125;

-- report 0026: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0026
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '27 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 126;

-- report 0027: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0027
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '28 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 127;

-- report 0028: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0028
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '29 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 128;

-- report 0029: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0029
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '30 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 129;

-- report 0030: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0030
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '31 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 130;

-- report 0031: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0031
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '32 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 131;

-- report 0032: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0032
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '33 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 132;

-- report 0033: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0033
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '34 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 133;

-- report 0034: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0034
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '35 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 134;

-- report 0035: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0035
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '36 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 135;

-- report 0036: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0036
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '37 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 136;

-- report 0037: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0037
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '38 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 137;

-- report 0038: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0038
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '39 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 138;

-- report 0039: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0039
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '40 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 139;

-- report 0040: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0040
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '41 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 140;

-- report 0041: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0041
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '42 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 141;

-- report 0042: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0042
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '43 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 142;

-- report 0043: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0043
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '44 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 143;

-- report 0044: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0044
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '45 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 144;

-- report 0045: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0045
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '46 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 145;

-- report 0046: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0046
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '47 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 146;

-- report 0047: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0047
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '48 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 147;

-- report 0048: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0048
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '49 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 148;

-- report 0049: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0049
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '50 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 149;

-- report 0050: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0050
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '51 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 150;

-- report 0051: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0051
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '52 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 151;

-- report 0052: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0052
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '53 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 152;

-- report 0053: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0053
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '54 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 153;

-- report 0054: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0054
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '55 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 154;

-- report 0055: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0055
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '56 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 155;

-- report 0056: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0056
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '57 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 156;

-- report 0057: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0057
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '58 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 157;

-- report 0058: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0058
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '59 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 158;

-- report 0059: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0059
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '60 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 159;

-- report 0060: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0060
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '61 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 160;

-- report 0061: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0061
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '62 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 161;

-- report 0062: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0062
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '63 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 162;

-- report 0063: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0063
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '64 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 163;

-- report 0064: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0064
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '65 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 164;

-- report 0065: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0065
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '66 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 165;

-- report 0066: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0066
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '67 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 166;

-- report 0067: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0067
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '68 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 167;

-- report 0068: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0068
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '69 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 168;

-- report 0069: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0069
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '70 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 169;

-- report 0070: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0070
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '71 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 170;

-- report 0071: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0071
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '72 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 171;

-- report 0072: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0072
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '73 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 172;

-- report 0073: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0073
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '74 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 173;

-- report 0074: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0074
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '75 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 174;

-- report 0075: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0075
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '76 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 175;

-- report 0076: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0076
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '77 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 176;

-- report 0077: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0077
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '78 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 177;

-- report 0078: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0078
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '79 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 178;

-- report 0079: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0079
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '80 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 179;

-- report 0080: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0080
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '81 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 180;

-- report 0081: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0081
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '82 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 181;

-- report 0082: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0082
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '83 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 182;

-- report 0083: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0083
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '84 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 183;

-- report 0084: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0084
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '85 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 184;

-- report 0085: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0085
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '86 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 185;

-- report 0086: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0086
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '87 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 186;

-- report 0087: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0087
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '88 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 187;

-- report 0088: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0088
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '89 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 188;

-- report 0089: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0089
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '90 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 189;

-- report 0090: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0090
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '1 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 190;

-- report 0091: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0091
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '2 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 191;

-- report 0092: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0092
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '3 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 192;

-- report 0093: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0093
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '4 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 193;

-- report 0094: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0094
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '5 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 194;

-- report 0095: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0095
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '6 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 195;

-- report 0096: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0096
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '7 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 196;

-- report 0097: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0097
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '8 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 197;

-- report 0098: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0098
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '9 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 198;

-- report 0099: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0099
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '10 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 199;

-- report 0100: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0100
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '11 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 200;

-- report 0101: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0101
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '12 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 201;

-- report 0102: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0102
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '13 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 202;

-- report 0103: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0103
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '14 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 203;

-- report 0104: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0104
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '15 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 204;

-- report 0105: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0105
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '16 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 205;

-- report 0106: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0106
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '17 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 206;

-- report 0107: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0107
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '18 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 207;

-- report 0108: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0108
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '19 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 208;

-- report 0109: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0109
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '20 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 209;

-- report 0110: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0110
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '21 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 210;

-- report 0111: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0111
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '22 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 211;

-- report 0112: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0112
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '23 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 212;

-- report 0113: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0113
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '24 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 213;

-- report 0114: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0114
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '25 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 214;

-- report 0115: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0115
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '26 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 215;

-- report 0116: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0116
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '27 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 216;

-- report 0117: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0117
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '28 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 217;

-- report 0118: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0118
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '29 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 218;

-- report 0119: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0119
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '30 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 219;

-- report 0120: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0120
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '31 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 220;

-- report 0121: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0121
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '32 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 221;

-- report 0122: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0122
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '33 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 222;

-- report 0123: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0123
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '34 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 223;

-- report 0124: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0124
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '35 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 224;

-- report 0125: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0125
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '36 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 225;

-- report 0126: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0126
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '37 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 226;

-- report 0127: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0127
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '38 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 227;

-- report 0128: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0128
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '39 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 228;

-- report 0129: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0129
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '40 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 229;

-- report 0130: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0130
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '41 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 230;

-- report 0131: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0131
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '42 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 231;

-- report 0132: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0132
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '43 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 232;

-- report 0133: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0133
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '44 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 233;

-- report 0134: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0134
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '45 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 234;

-- report 0135: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0135
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '46 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 235;

-- report 0136: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0136
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '47 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 236;

-- report 0137: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0137
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '48 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 237;

-- report 0138: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0138
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '49 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 238;

-- report 0139: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0139
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '50 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 239;

-- report 0140: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0140
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '51 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 240;

-- report 0141: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0141
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '52 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 241;

-- report 0142: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0142
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '53 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 242;

-- report 0143: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0143
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '54 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 243;

-- report 0144: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0144
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '55 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 244;

-- report 0145: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0145
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '56 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 245;

-- report 0146: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0146
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '57 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 246;

-- report 0147: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0147
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '58 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 247;

-- report 0148: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0148
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '59 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 248;

-- report 0149: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0149
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '60 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 249;

-- report 0150: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0150
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '61 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 250;

-- report 0151: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0151
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '62 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 251;

-- report 0152: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0152
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '63 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 252;

-- report 0153: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0153
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '64 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 253;

-- report 0154: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0154
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '65 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 254;

-- report 0155: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0155
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '66 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 255;

-- report 0156: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0156
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '67 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 256;

-- report 0157: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0157
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '68 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 257;

-- report 0158: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0158
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '69 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 258;

-- report 0159: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0159
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '70 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 259;

-- report 0160: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0160
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '71 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 260;

-- report 0161: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0161
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '72 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 261;

-- report 0162: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0162
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '73 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 262;

-- report 0163: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0163
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '74 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 263;

-- report 0164: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0164
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '75 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 264;

-- report 0165: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 13) AS total_0165
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '76 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 265;

-- report 0166: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 14) AS total_0166
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '77 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 266;

-- report 0167: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 15) AS total_0167
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '78 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 267;

-- report 0168: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 16) AS total_0168
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '79 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 268;

-- report 0169: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 17) AS total_0169
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '80 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 269;

-- report 0170: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 1) AS total_0170
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '81 days'
  AND o.status IN ('open', 'held', 'state_5')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 270;

-- report 0171: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 2) AS total_0171
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '82 days'
  AND o.status IN ('open', 'held', 'state_6')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 271;

-- report 0172: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 3) AS total_0172
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '83 days'
  AND o.status IN ('open', 'held', 'state_7')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 272;

-- report 0173: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 4) AS total_0173
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '84 days'
  AND o.status IN ('open', 'held', 'state_8')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 273;

-- report 0174: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 5) AS total_0174
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
WHERE t.created_at >= now() - interval '85 days'
  AND o.status IN ('open', 'held', 'state_9')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 274;

-- report 0175: orders joined to products
SELECT t.id, t.created_at, o.name, sum(t.amount * 6) AS total_0175
FROM orders AS t
JOIN products AS o ON o.id = t.products_id
WHERE t.created_at >= now() - interval '86 days'
  AND o.status IN ('open', 'held', 'state_10')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 275;

-- report 0176: customers joined to shipments
SELECT t.id, t.created_at, o.name, sum(t.amount * 7) AS total_0176
FROM customers AS t
JOIN shipments AS o ON o.id = t.shipments_id
WHERE t.created_at >= now() - interval '87 days'
  AND o.status IN ('open', 'held', 'state_0')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 1
ORDER BY total DESC
LIMIT 276;

-- report 0177: order_lines joined to invoices
SELECT t.id, t.created_at, o.name, sum(t.amount * 8) AS total_0177
FROM order_lines AS t
JOIN invoices AS o ON o.id = t.invoices_id
WHERE t.created_at >= now() - interval '88 days'
  AND o.status IN ('open', 'held', 'state_1')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 2
ORDER BY total DESC
LIMIT 277;

-- report 0178: products joined to payments
SELECT t.id, t.created_at, o.name, sum(t.amount * 9) AS total_0178
FROM products AS t
JOIN payments AS o ON o.id = t.payments_id
WHERE t.created_at >= now() - interval '89 days'
  AND o.status IN ('open', 'held', 'state_2')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 3
ORDER BY total DESC
LIMIT 278;

-- report 0179: shipments joined to orders
SELECT t.id, t.created_at, o.name, sum(t.amount * 10) AS total_0179
FROM shipments AS t
JOIN orders AS o ON o.id = t.orders_id
WHERE t.created_at >= now() - interval '90 days'
  AND o.status IN ('open', 'held', 'state_3')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 4
ORDER BY total DESC
LIMIT 279;

-- report 0180: invoices joined to customers
SELECT t.id, t.created_at, o.name, sum(t.amount * 11) AS total_0180
FROM invoices AS t
JOIN customers AS o ON o.id = t.customers_id
WHERE t.created_at >= now() - interval '1 days'
  AND o.status IN ('open', 'held', 'state_4')
GROUP BY t.id, t.created_at, o.name
HAVING count(*) > 0
ORDER BY total DESC
LIMIT 280;

-- report 0181: payments joined to order_lines
SELECT t.id, t.created_at, o.name, sum(t.amount * 12) AS total_0181
FROM payments AS t
JOIN order_lines AS o ON o.id = t.order_lines_id
