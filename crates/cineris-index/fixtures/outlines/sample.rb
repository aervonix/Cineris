# frozen_string_literal: true

require "json"

# Parses configuration files.
module Config
  VERSION = "1.2.0"

  # A loaded configuration.
  class Loader < Base
    attr_reader :path

    def initialize(path)
      @path = path
    end

    # Reads the file.
    def self.read(path)
      new(path).tap do |loader|
        loader.load!
      end
    end

    def load!
      text = <<~TEXT
        def not_a_method
        end
      TEXT
      JSON.parse(text)
    end

    def ==(other) = path == other.path

    class << self
      def default
        new("config.json")
      end
    end

    private

    def secret; end
  end
end

=begin
def commented_out
end
=end

def top_level_helper
  :ok
end
