#pragma once

#include <string>
#include <vector>

namespace shop {

/// A line of an order.
struct Line {
    std::string sku;
    int count = 0;
};

class Order final : public Base {
public:
    /// Makes an empty order.
    explicit Order(int id);
    ~Order();

    std::vector<Line> lines() const;
    template <typename F>
    void each(F&& f) const {
        for (const auto& line : lines_) { f(line); }
    }
    static Order empty();

private:
    int id_;
    std::vector<Line> lines_;
};

using Orders = std::vector<Order>;

int total(const Orders& orders);

namespace detail {
inline int clamp(int value) { return value < 0 ? 0 : value; }
}  // namespace detail

}  // namespace shop

extern "C" {
int c_api_version(void);
}

std::vector<shop::Line> shop::Order::lines() const {
    return lines_;
}
