using System;
using System.Collections.Generic;

namespace Shop.Billing
{
    /// <summary>
    /// Works out invoices.
    /// </summary>
    public sealed class InvoiceService : IInvoiceService
    {
        private readonly Dictionary<string, int> totals = new Dictionary<string, int> { };
        public const int Limit = 10;

        /// Makes a service.
        public InvoiceService(ILogger logger)
        {
            _logger = logger;
        }

        public int Count { get; private set; }

        public string Name => "invoices";

        [Obsolete("old")]
        public async Task<Invoice> CreateAsync(Order order, CancellationToken token = default)
        {
            var path = @"C:\invoices\";
            if (order == null) { throw new ArgumentNullException(nameof(order)); }
            return await Task.FromResult(new Invoice());
        }

        private static decimal Round(decimal value) => Math.Round(value, 2);

        public event EventHandler Created;
    }

    public interface IInvoiceService
    {
        Task<Invoice> CreateAsync(Order order, CancellationToken token);
    }

    public record struct Money(decimal Amount, string Currency);

    public enum Status { Draft, Sent }

    public delegate void Handler(object sender, EventArgs args);

    internal struct Point
    {
        public int X;
        public override string ToString() { return $"{X}"; }
    }
}
