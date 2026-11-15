package com.example.shop;

import java.util.List;
import java.util.Map;

/**
 * A shopping cart.
 */
public final class Cart<T extends Item> implements Iterable<T> {
    private final List<T> items = new ArrayList<>();
    private static final int LIMIT = 50;

    /** Makes an empty cart. */
    public Cart() {
        this(LIMIT);
    }

    Cart(int limit) {
        if (limit < 0) {
            throw new IllegalArgumentException("limit");
        }
    }

    /**
     * Adds an item.
     * @param item what to add
     */
    @Override
    public synchronized boolean add(T item) {
        for (T other : items) {
            if (other.equals(item)) { return false; }
        }
        return items.add(item);
    }

    public Map<String, List<T>> byCategory(
            String prefix,
            int max) throws IOException {
        Runnable r = new Runnable() {
            public void run() {}
        };
        return null;
    }

    public static <U> Cart<U> empty() {
        return new Cart<>();
    }

    interface Listener {
        void changed(Cart<?> cart);
    }

    enum State {
        OPEN, CLOSED;

        boolean isOpen() { return this == OPEN; }
    }

    record Line(String sku, int count) {}
}

@interface Audited {
    String value();
}
