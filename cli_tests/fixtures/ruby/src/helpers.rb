module Greetings
  module Formatting
    def self.wrap(value)
      "[#{value}]"
    end

    def self.empty; end
  end
end
