<?php

namespace App\Billing;

use App\Models\Order;

/**
 * Works out invoices.
 */
final class InvoiceService implements Billable
{
    public const LIMIT = 10;
    private array $cache = [];

    /** Makes a service. */
    public function __construct(private Logger $logger)
    {
    }

    public static function create(Order $order): Invoice
    {
        $sql = <<<SQL
            function fake() {
        SQL;
        return new Invoice($order);
    }

    #[Deprecated]
    protected function &reference(): array
    {
        return $this->cache;
    }
}

interface Billable
{
    public function bill(): void;
}

trait Logs
{
    function log(string $message) {}
}

enum Status: string
{
    case Draft = 'draft';
}

function helper($x) {
    return $x; # a comment with function fake() {
}
