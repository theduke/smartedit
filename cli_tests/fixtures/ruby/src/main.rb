module Greetings
  class Greeter
    def initialize(name)
      @name = name
    end

    def greet(punctuation = "!")
      "Hello, #{@name}#{punctuation}"
    end

    def self.default
      new("world")
    end

    def self.label
      "default"
    end

    class << self
      def version
        "1.0"
      end
    end
  end
end

def compatibility_label
  Greetings::Greeter.default.greet
end
